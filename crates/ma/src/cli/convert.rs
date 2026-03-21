use std::fs;

use clap::Args;

use crate::{
    etl::{
        EtlFormat, build_label_map, convert_to_npz, label_map_from_records, output_paths,
        read_etl_dir, read_etl_file,
    },
    kanjivg,
};

/// Parse a list of single-character tokens into `char`s for use in filtering.
///
/// Each token must be exactly one Unicode character; multi-character tokens are
/// warned about and ignored. So don't pass in ~ from the shell again dumdum.
/// Use single quotes like a non heretic.
fn parse_chars(tokens: &[String]) -> Vec<char> {
    tokens
        .iter()
        .filter_map(|t| {
            let mut cs = t.chars();
            let ch = cs.next()?;
            if cs.next().is_some() {
                tracing::warn!("ignoring token {t:?} not a char");
                return None;
            }
            Some(ch)
        })
        .collect()
}

// I'm leaving the ETL conversion crap in here even though the data it contains
// is too noisy to be useful. It might come in handy later. Or future me will
// rip it out in anger.
#[derive(Args)]
pub struct ConvertArgs {
    /// Path to a single ETL file, or a directory of ETL files to
    /// process in sorted order
    #[arg(short, long)]
    input: String,

    /// ETL binary formats this was even attemped upon:
    ///   etl1 / etl7        M-type katakana/hiragana  source 64x63, 4bpp
    ///   etl8b / etl9b      8B-type hiragana+kanji    source 64x63, 1bpp
    ///   etl8g / etl9g      8G-type hiragana+kanji    source 128x127, 4bpp
    ///
    /// SVG formats:
    ///   kanjivg            KanjiVG stroke SVGs. Abused different stroke sizes to make synthetic data
    #[arg(short, long)]
    format: String,

    /// Directory to write training source npz files, mkdir iff missing
    ///
    /// Outputs:
    /// without --split:  {out}/imgs.npz  and  {out}/labels.npz
    /// with    --split:  {out}/train-imgs.npz, {out}/train-labels.npz,
    ///                   {out}/test-imgs.npz,  {out}/test-labels.npz
    #[arg(short, long, default_value = ".")]
    out: String,

    /// Fraction of records to use for *training* without --split its everything
    /// to a single imgs/labels pair npz file.
    /// --split 0.8 means 80% for training 20% test
    #[arg(long, default_value_t = 0.0)]
    train_split: f64,

    /// Comma-separated Unicode characters to include, was here more for the ETL
    /// stuff but if I start using merged npz files this might be more useful
    /// again at merging datasets together if they have duplicate keys. ex: "あ,
    /// い,う,え,お" restricts output to those five classes. If omitted every
    /// decodable character is included and assigned a sequential label index
    /// sorted by Unicode code point. I'm presuming image == unicode code point
    /// for my use case.
    #[arg(short, long)]
    chars: Option<String>,

    /// RNG seed for the per-character train/test sample selection.
    ///
    /// Without --seed the split is deterministic but always assigns the first
    /// train_per_char augmentation samples to train and the remaining to test.
    ///
    /// This can be problematic if say your good test data is at the end.
    ///
    /// With --seed the samples for each character are shuffled using a
    /// reproducible RNG seeded from this args value before the split, so the
    /// clean and augmented variants in this example are randomly distributed
    /// between train and test. Re-running with the same seed produces identical
    /// output however to remain deterministic between runs and not entirely
    /// random. Note if the underlying data changes all bets are off.
    #[arg(long)]
    seed: Option<u64>,

    /// RNG seed for geometric augmentation perturbations (kanjivg only).
    ///
    /// Without --aug-seed each convert run produces different rotations,
    /// translations and scales even when --seed is fixed, because augmentation
    /// uses a fresh non-deterministic RNG.
    ///
    /// With --aug-seed the augmentation RNG is seeded from this value so the
    /// exact same pixel perturbations are reproduced on every run with the same
    /// value. Useful for debugging augmentation issues or exact reproducibility.
    /// --seed and --aug-seed are independent and can be combined freely.
    #[arg(long)]
    aug_seed: Option<u64>,

    // TODO: a lot of this kanjivg only crap will filter out to other datasets
    // at some point.
    /// Characters to exclude from the dataset (kanjivg only).
    ///
    /// Each flag value must be one Unicode character. Repeat the flag
    /// for each character to exclude like sooooo:
    ///
    ///   --filter ! --filter , --filter ~
    ///
    /// Characters that match are dropped before label assignment so the
    /// remaining classes are always contiguous.
    #[arg(long)]
    filter: Vec<String>,

    /// Exclude characters whose Unicode name contains the substring (kanjivg only).
    ///
    /// Matching is case-insensitive and is simple substring matching. Supply as a
    /// comma-separated list in one flag, repeated flags, or both; a character
    /// is excluded when its Unicode name contains ANY of the provided strings:
    ///
    ///   --filter-name "letter small,cjk radical"
    ///   --filter-name "letter small" --filter-name "cjk radical"
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    filter_name: Vec<String>,
}

// TODO: stop being lazy and abuse clap for arg validation
pub fn run(args: ConvertArgs) {
    crate::cli::init_tracing();

    if !(0.0..1.0).contains(&args.train_split) {
        eprintln!(
            "--train-split must be in [0.0, 1.0), got {}",
            args.train_split
        );
        std::process::exit(1);
    }

    fs::create_dir_all(&args.out).unwrap_or_else(|e| {
        eprintln!("cannot create output directory {}: {e}", args.out);
        std::process::exit(1);
    });

    let paths = output_paths(&args.out, args.train_split);

    if args.format.to_lowercase() == "kanjivg" {
        let filter_chars = parse_chars(&args.filter);
        let filter_names: Vec<String> = args.filter_name.iter().map(|s| s.to_lowercase()).collect();
        kanjivg::convert_kanjivg_dir(
            &args.input,
            &paths,
            args.train_split,
            args.seed,
            args.aug_seed,
            &filter_chars,
            &filter_names,
        )
        .unwrap_or_else(|e| {
            eprintln!("kanjivg conversion failed: {e}");
            std::process::exit(1);
        });
        return;
    }

    // TODO: Future mitch this is yeetable out of the entire code base unless
    // ETL is eventually useful?
    let format = EtlFormat::from_str(&args.format).unwrap_or_else(|| {
        eprintln!(
            "unknown format {:?} valid values: etl1, etl7, etl8b, etl9b, etl8g, etl9g",
            args.format
        );
        std::process::exit(1);
    });

    let input_path = std::path::Path::new(&args.input);
    let records = if input_path.is_dir() {
        read_etl_dir(&args.input, format)
    } else {
        read_etl_file(&args.input, format)
    }
    .unwrap_or_else(|e| {
        eprintln!("failed to read ETL data from {}: {e}", args.input);
        std::process::exit(1);
    });

    tracing::info!(total = records.len(), "records loaded");

    let label_map = if let Some(chars_str) = &args.chars {
        let chars: Vec<char> = chars_str
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                let mut cs = s.chars();
                let ch = cs.next()?;
                if cs.next().is_none() {
                    Some(ch)
                } else {
                    tracing::warn!("ignoring multi-char token {s:?} in --chars");
                    None
                }
            })
            .collect();
        tracing::debug!(n = chars.len(), "using user-supplied char filter");
        build_label_map(&chars)
    } else {
        let map = label_map_from_records(&records);
        tracing::info!(classes = map.len(), "auto-detected label map from records");
        map
    };

    convert_to_npz(&records, &paths, &label_map, args.train_split).unwrap_or_else(|e| {
        eprintln!("failed to write npz: {e}");
        std::process::exit(1);
    });
}
