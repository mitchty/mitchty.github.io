use burn::{
    backend::{Autodiff, Wgpu},
    data::dataset::{Dataset, vision::MnistDataset},
    optim::AdamConfig,
};
use clap::Args;

use crate::{
    data::{ConcatNpzDataset, DataItem, KaggleMnistDataset},
    model::ModelConfig,
    training::{self, TrainingConfig},
};

#[derive(Args, Default)]
pub struct TrainArgs {
    /// Directory to write model artifacts and config crap into
    #[arg(short, long, default_value = "ma")]
    output: String,

    /// Directory layout produced by `ma convert --train-split`.
    ///
    /// When given, any of --train-imgs / --train-labels / --test-imgs /
    /// --test-labels / --classmap that are *not* explicitly supplied are filled
    /// in with their standard names inside this directory:
    ///
    ///   {input}/train-imgs.npz
    ///   {input}/train-labels.npz
    ///   {input}/test-imgs.npz
    ///   {input}/test-labels.npz
    ///   {input}/classmap.json
    ///
    /// Explicit flags always take precedence over auto-discovered paths,
    /// so you can mix: --input /data/kanjivg --train-imgs /other/train-imgs.npz
    /// will use /other/train-imgs.npz for images but auto-fill everything else.
    ///
    /// I did this cause I got sick of specifying 5 args constantly to train.
    /// --input and let `ma` figure it out and bitch if it can't find what it
    /// looks for. I can revisit this later.
    #[arg(long)]
    input: Option<String>,

    // TODO: This barely worked but keeping it for now
    /// Path to a kaggle style mnist csv file
    #[arg(short, long)]
    file: Option<String>,

    /// What fraction of the csv to use for training. Remainder becomes
    /// validation set. default: 0.8 only useful with file data
    #[arg(short, long)]
    split: Option<f64>,

    // Npz dataset inputs, each flag can be repeated to supply multiple files to
    // combine datasets into a single npz file All imgs/labels lists must be the
    // same length and share a label space for now. This barely works.
    //
    // Note the 28x28 is to match the mnist stuff. This might not be worth
    // keeping long. Or I might want to make them more accurate.
    /// Training images npzs are u8 arrays (N, 28, 28)
    /// ex concatenate: --train-imgs a.npz --train-imgs b.npz
    #[arg(long, num_args = 1..)]
    train_imgs: Vec<String>,

    /// Training labels npzs: u8 array shape (N,) Must pair with --train-imgs for now cause I'm a hack
    #[arg(long, num_args = 1..)]
    train_labels: Vec<String>,

    /// Test or validation image npzs: u8 array shape (M, 28, 28).
    #[arg(long, num_args = 1..)]
    test_imgs: Vec<String>,

    /// Test or validation labels npzs: u8 array shape (M,) Must pair with --test-imgs here too
    #[arg(long, num_args = 1..)]
    test_labels: Vec<String>,

    /// Number of training epochs. Default: 30, number was more for testing kuzushiji 49 dataset its low for really high counts.
    #[arg(short, long)]
    epochs: Option<usize>,

    /// Mini-batch size. Larger values use more VRAM but train faster and
    /// produce more stable gradient estimates. Default: 64.
    // TODO: note 256 seems a better default but not sure I should use it yet
    // 512 fails on my macbook, works on 4090
    #[arg(short, long)]
    batch_size: Option<usize>,

    /// How many dataloader worker threads. Set to 0 to load on the main thread Default: 4. Unsure of a good value the input data is small right now.
    #[arg(short, long)]
    workers: Option<usize>,

    /// Adam learning rate. Default: 1e-4.
    #[arg(long)]
    lr: Option<f64>,

    /// Hidden layer width for two fully-connected layers being trained.
    /// Larger values increase model capacity but also training time.
    /// Default: 512.
    #[arg(long)]
    hidden_size: Option<usize>,

    // TODO: I've just been playing with this to abuse my 4090, haven't tried
    // optimizing these values at all very much. But got training time to an
    // hour or so.
    /// Base conv channel count. Channels grow as C to 2C to 4C across the three
    /// conv blocks. Default: 32 = 32 -> 64 -> 128
    /// Doubling to 64 = 64 -> 128 -> 256 roughly 4x's the conv FLOPS and is the
    /// main knob for filling GPU compute beyond batch size. Future mitch pay
    /// attention after you forget this in a week.
    #[arg(long)]
    conv_channels: Option<usize>,

    /// Dropout rate applied after conv2, conv3, and the hidden linear layer are applied.
    /// Default: 0.5.
    /// Lower values ex: 0.3 help when the dataset is large
    /// and well-augmented; higher values add regularization for smaller data sets.
    ///
    // TODO: For the stuff I'm trying 0.3 seems a decent size but I haven't tuned it.
    #[arg(long)]
    dropout: Option<f64>,

    /// Per-pixel normalization mean: (pixel/255 - mean) / std.
    /// Must match the dataset. K49=0.1793, KMNIST=0.1904, MNIST=0.1307.
    /// Default: 0.1793 (K49).
    // TODO: For the stuff in convert I calculaate and dump this into the
    // config.json and we use that. This is kinda here only for manual tuning or
    // to be a jerk
    #[arg(long)]
    norm_mean: Option<f64>,

    /// Per-pixel normalization std.
    /// K49=0.3416, KMNIST=0.3475, MNIST=0.3081.  Default: 0.3416 (K49).
    // TODO: Like ^^^ here more for legacy testing
    #[arg(long)]
    norm_std: Option<f64>,

    /// Path to classmap.json file written by `ma convert`.
    /// Embeds the class-index to Unicode-character mapping in config.json so
    /// the inference engine can label results without a separate file lying about.
    #[arg(long)]
    classmap: Option<String>,
}

/// Thin wrapper that converts burn's `MnistDataset` to our
/// `DataItem` (label: u32) so the built-in MNIST fallback path "just works" with
/// the `Dataset<DataItem>` bound required by `training::train`.
// Burn is still new to me and I'm abusing the mnist background like a hack for
// things it probably shouldn't be used for.
struct MnistAdapter(MnistDataset);

impl burn::data::dataset::Dataset<DataItem> for MnistAdapter {
    fn get(&self, index: usize) -> Option<DataItem> {
        self.0.get(index).map(|item| DataItem {
            image: item.image,
            label: item.label as u32,
        })
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub fn run(args: TrainArgs) {
    type MyBackend = Wgpu<f32, i32>;
    type MyAutodiffBackend = Autodiff<MyBackend>;

    // Resolve --input directory into individual default flags it stands for.
    // Any flag that was supplied explicitly is accepted as right over this.
    // --input only fills any omissions. We do this first so the rest of run()
    // is oblivious to where the paths came from.
    let mut args = args;
    if let Some(ref dir) = args.input.clone() {
        let p = |name: &str| -> String { format!("{dir}/{name}") };

        let candidates = [
            ("train-imgs.npz", "--train-imgs"),
            ("train-labels.npz", "--train-labels"),
            ("test-imgs.npz", "--test-imgs"),
            ("test-labels.npz", "--test-labels"),
        ];

        // Warn about any candidate that doesn't exist on disk so the I know
        // whats busted earlier than the npz parse error we would get otherwise.
        for (name, flag) in &candidates {
            let path = p(name);
            if !std::path::Path::new(&path).exists() {
                eprintln!(
                    "warning: --input auto-fill: {path} not found \
                     (override with {flag} to suppress this warning)"
                );
            }
        }

        if args.train_imgs.is_empty() {
            args.train_imgs = vec![p("train-imgs.npz")];
        }
        if args.train_labels.is_empty() {
            args.train_labels = vec![p("train-labels.npz")];
        }
        if args.test_imgs.is_empty() {
            args.test_imgs = vec![p("test-imgs.npz")];
        }
        if args.test_labels.is_empty() {
            args.test_labels = vec![p("test-labels.npz")];
        }

        // classmap is Option<String> so only fill if not already set and the file exists.
        if args.classmap.is_none() {
            let cm = p("classmap.json");
            if std::path::Path::new(&cm).exists() {
                args.classmap = Some(cm);
            } else {
                tracing::warn!(
                    path = cm,
                    "--input auto-fill: classmap.json not found, training without classmap"
                );
            }
        }
    }

    let device = burn::backend::wgpu::WgpuDevice::default();
    let artifact_dir = args.output.as_str();

    let classmap: Vec<String> = if let Some(path) = &args.classmap {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("cannot read classmap {path}: {e}");
            std::process::exit(1);
        });
        serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("cannot parse classmap {path}: {e}");
            std::process::exit(1);
        })
    } else {
        Vec::new()
    };

    // Auto-load stats.json data from the first --train-imgs file unless
    // --norm-mean / --norm-std are provided.
    let auto_stats: Option<(f64, f64)> = if args.norm_mean.is_none() && args.norm_std.is_none() {
        args.train_imgs
            .first()
            .and_then(|p| std::path::Path::new(p).parent())
            .and_then(|dir| std::fs::read_to_string(dir.join("stats.json")).ok())
            .and_then(|text| {
                let v: serde_json::Value = serde_json::from_str(&text).ok()?;
                let mean = v["norm_mean"].as_f64()?;
                let std = v["norm_std"].as_f64()?;
                Some((mean, std))
            })
    } else {
        None
    };

    if let Some((mean, std)) = auto_stats {
        tracing::info!(mean, std, "loaded normalization stats from stats.json");
    }

    // Burn TrainingConfig is whats driving most of this args switches.
    let hidden_size = args.hidden_size.unwrap_or(512);
    let conv_channels = args.conv_channels.unwrap_or(32);
    let dropout = args.dropout.unwrap_or(0.5);
    let with_config = |mut config: TrainingConfig| {
        if let Some(v) = args.epochs {
            config.num_epochs = v;
        }
        if let Some(v) = args.batch_size {
            config.batch_size = v;
        }
        if let Some(v) = args.workers {
            config.num_workers = v;
        }
        if let Some(v) = args.lr {
            config.learning_rate = v;
        }

        // Iff these args exist, they win over whatever is in the stats.json
        // file from conversion.
        match (args.norm_mean, args.norm_std, auto_stats) {
            (Some(m), Some(s), _) => {
                config.norm_mean = m;
                config.norm_std = s;
            }
            (Some(m), None, _) => {
                config.norm_mean = m;
            }
            (None, Some(s), _) => {
                config.norm_std = s;
            }
            (None, None, Some((m, s))) => {
                config.norm_mean = m;
                config.norm_std = s;
            }
            (None, None, None) => {} // Defaults here I guess whatever
        }
        if !classmap.is_empty() {
            config.class_map = classmap.clone();
        }
        config
    };
    let with_epochs = with_config;

    let npz_mode = !args.train_imgs.is_empty();

    if npz_mode {
        // Be sure that everything matches. Need to use a prooper error library
        // here but got lazy and this codes not critical so I got lazy and quit
        // caring a bit on stuff that only runs like once. Exit is fine for this
        // use case.
        for (name, list) in [
            ("--train-imgs", &args.train_imgs),
            ("--train-labels", &args.train_labels),
            ("--test-imgs", &args.test_imgs),
            ("--test-labels", &args.test_labels),
        ] {
            if list.is_empty() {
                eprintln!("{name} is required when an npz flag is supplied");
                std::process::exit(1);
            }
        }
        // TODO: again gotta yeet this into clap arg parsing at some point
        if args.train_imgs.len() != args.train_labels.len() {
            eprintln!(
                "--train-imgs {} and --train-labels {} must have the same count of classes",
                args.train_imgs.len(),
                args.train_labels.len()
            );
            std::process::exit(1);
        }
        if args.test_imgs.len() != args.test_labels.len() {
            eprintln!(
                "--test-imgs {} and --test-labels {} must have the same count of classes",
                args.test_imgs.len(),
                args.test_labels.len()
            );
            std::process::exit(1);
        }

        let train_imgs: Vec<&str> = args.train_imgs.iter().map(String::as_str).collect();
        let train_labels: Vec<&str> = args.train_labels.iter().map(String::as_str).collect();
        let test_imgs: Vec<&str> = args.test_imgs.iter().map(String::as_str).collect();
        let test_labels: Vec<&str> = args.test_labels.iter().map(String::as_str).collect();

        let train_ds = ConcatNpzDataset::from_pairs(&train_imgs, &train_labels);
        let test_ds = ConcatNpzDataset::from_pairs(&test_imgs, &test_labels);
        let num_classes = train_ds.num_classes();

        tracing::info!(
            num_classes,
            train_files = args.train_imgs.len(),
            test_files = args.test_imgs.len(),
            train_items = train_ds.len(),
            test_items = test_ds.len(),
            "npz mode"
        );

        let config = with_epochs(TrainingConfig::new(
            ModelConfig::new(num_classes, hidden_size)
                .with_conv_channels(conv_channels)
                .with_dropout(dropout),
            AdamConfig::new(),
        ));
        training::train::<MyAutodiffBackend, _>(artifact_dir, config, device, train_ds, test_ds);
    } else if let Some(path) = &args.file {
        // Kaggle CSV mode, I doubt this works anymore
        let fraction = args.split.unwrap_or(0.8);
        let (train_ds, test_ds) = KaggleMnistDataset::from_csv(path).split(fraction);
        let config = with_epochs(TrainingConfig::new(
            ModelConfig::new(10, hidden_size)
                .with_conv_channels(conv_channels)
                .with_dropout(dropout),
            AdamConfig::new(),
        ));
        training::train::<MyAutodiffBackend, _>(artifact_dir, config, device, train_ds, test_ds);
    } else {
        // Fallback use burn's built-in MNIST wrapped to produce DataItems to
        // train against this is all example code that likely isn't useful
        let config = with_epochs(TrainingConfig::new(
            ModelConfig::new(10, hidden_size)
                .with_conv_channels(conv_channels)
                .with_dropout(dropout),
            AdamConfig::new(),
        ));
        training::train::<MyAutodiffBackend, _>(
            artifact_dir,
            config,
            device,
            MnistAdapter(MnistDataset::train()),
            MnistAdapter(MnistDataset::test()),
        );
    }
}
