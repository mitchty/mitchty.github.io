use burn::backend::Wgpu;
use clap::Args;

use crate::inference;

#[derive(Args)]
pub struct InferArgs {
    /// Directory containing trained model artifacts and config
    #[arg(short, long, default_value = "ma")]
    output: String,

    /// Path to a kaggle style mnist csv file
    #[arg(short, long)]
    file: Option<String>,

    /// Path to an npz image file u8 array shape (N, 28, 28)
    #[arg(long)]
    imgs: Option<String>,

    /// Path to an npz labels file u8 array shape (N,)
    #[arg(long)]
    labels: Option<String>,

    /// Dataset index of the item to infer off of default: 4
    #[arg(short, long, default_value_t = 4)]
    index: usize,
}

pub fn run(args: InferArgs) {
    crate::cli::init_tracing();
    type MyBackend = Wgpu<f32, i32>;

    let device = burn::backend::wgpu::WgpuDevice::default();
    let artifact_dir = args.output.as_str();

    match (&args.imgs, &args.labels, &args.file) {
        (Some(imgs), Some(labels), _) => {
            inference::infer_from_npz::<MyBackend>(artifact_dir, device, imgs, labels, args.index);
        }

        (_, _, Some(path)) => {
            inference::infer_from_file::<MyBackend>(artifact_dir, device, path, args.index);
        }

        // Not sure how long I keep the default mnist stuff around or not
        _ => {
            use burn::data::dataset::{Dataset, vision::MnistDataset};
            let item = MnistDataset::test().get(args.index).unwrap_or_else(|| {
                panic!(
                    "requested index {} out of range in built-in burn mnist test set",
                    args.index
                )
            });
            inference::infer::<MyBackend>(artifact_dir, device, item);
        }
    }
}
