use std::fs;

use clap::Args;

use crate::{
    etl::{
        EtlFormat, build_label_map, convert_etlcdb, convert_to_npz, label_map_from_records,
        max_batch_dims, output_paths, read_etl_dir, read_etl_file, read_etlcdb_dir,
        write_merged_etlcdb,
    },
    filter_config::FilterConfig,
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

    /// ETL binary formats (single dataset):
    ///   etl1 / etl6 / etl7   M-type katakana/hiragana  64x63, 4bpp, JIS X 0201
    ///   etl8b                8B-type hiragana+kanji    64x63, 1bpp, JIS X 0208  (512 bytes/rec)
    ///   etl9b                9B-type hiragana+kanji    64x63, 1bpp, JIS X 0208  (576 bytes/rec)
    ///   etl8g                8G-type hiragana+kanji    128x127, 4bpp, JIS X 0208 (image @ byte 61)
    ///   etl9g                9G-type hiragana+kanji    128x127, 4bpp, JIS X 0208 (image @ byte 65)
    ///
    /// ETL combined mode (all supported datasets at once):
    ///   etlcdb               Root directory containing ETL1...ETL9G subdirectories.
    ///                        Reads ETL1, ETL6, ETL7 (M-type), ETL2 (K-type),
    ///                        ETL3/4/5 (C-type), ETL8B (B8), ETL9B (B9),
    ///                        ETL8G (G8), ETL9G (G9) and writes per-family npz files:
    ///                          {out}/m-imgs.npz, {out}/k-imgs.npz, {out}/c-imgs.npz,
    ///                          {out}/b-imgs.npz, {out}/g-imgs.npz
    ///                        plus a shared classmap.json and stats.json.
    ///                        Use --exclude to skip specific datasets (e.g., --exclude ETL2).
    ///
    /// SVG formats:
    ///   kanjivg              KanjiVG stroke SVGs, augmented with varying stroke widths
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

    /// (etlcdb only) Merge every format family into a single imgs.npz +
    /// labels.npz, identical in layout to a kanjivg conversion.
    ///
    /// All images are upscaled to the largest native resolution in the dataset
    /// (128x127 for a full ETLcdb run) so no detail is thrown away.
    /// M/B-type images (64x63) are bilinearly upscaled; G-type (128x127)
    /// images are stored as-is.
    ///
    /// Without this flag etlcdb writes per-family files at native resolution:
    ///   {out}/m-imgs.npz  {out}/b-imgs.npz  {out}/g-imgs.npz
    #[arg(long, default_value_t = false)]
    merge: bool,

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
    /// Path to a YAML filter configuration file.
    ///
    /// Loads `chars` (individual characters to exclude) and `names`
    /// (Unicode-name substrings to exclude) from a YAML file so you don't
    /// have to repeat a wall of --filter / --filter-name flags on every run.
    ///
    /// Example file:
    ///
    ///   chars:
    ///     - "!"
    ///     - "~"
    ///   names:
    ///     - halfwidth
    ///     - katakana iteration
    ///     - middle dot
    ///
    /// Config values are used as the base; any --filter / --filter-name
    /// arguments on the command line are appended on top of the file values.
    /// Works for etlcdb and kanjivg.
    #[arg(long)]
    filter_config: Option<String>,

    /// Characters to exclude from the dataset.
    ///
    /// Each flag value must be one Unicode character. Repeat the flag
    /// for each character to exclude like sooooo:
    ///
    ///   --filter ! --filter , --filter ~
    ///
    /// Characters that match are dropped before label assignment so the
    /// remaining classes are always contiguous. Works for etlcdb and kanjivg.
    #[arg(long)]
    filter: Vec<String>,

    /// ETL files/datasets to exclude from processing (etlcdb mode only).
    ///
    /// Specify dataset directory names (e.g., "ETL2") to exclude all files
    /// in that dataset, or specific filenames (e.g., "ETL2_1") to exclude
    /// individual files. Full paths are also supported for precise matching.
    /// Can be specified multiple times:
    ///
    ///   --exclude ETL2              # exclude entire ETL2 directory
    ///   --exclude ETL2_1            # exclude specific file ETL2_1
    ///   --exclude /path/to/ETL2_1   # exclude by full path
    ///
    /// Works for etlcdb mode only.
    #[arg(long)]
    exclude: Vec<String>,

    /// Exclude characters whose Unicode name contains the substring.
    ///
    /// Matching is case-insensitive and is simple substring matching. Supply as a
    /// comma-separated list in one flag, repeated flags, or both; a character
    /// is excluded when its Unicode name contains ANY of the provided strings:
    ///
    ///   --filter-name "letter small,cjk radical"
    ///   --filter-name "letter small" --filter-name "cjk radical"
    ///
    /// Works for etlcdb and kanjivg.
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    filter_name: Vec<String>,

    /// Characters to include as a whitelist. Only these characters survive, all
    /// others are dropped before label assignment when in the whitelist.
    ///
    /// Each flag value must be exactly one Unicode character/codepoint. Repeat
    /// the flag for each character:
    ///
    ///   --include ア --include イ --include ウ
    ///
    /// Composes with --filter: blacklisted characters are still removed even if
    /// they appear in the include list. An empty include list means "include
    /// everything" (no whitelist applied). Works for etlcdb and kanjivg.
    #[arg(long)]
    include: Vec<String>,

    /// Include only characters whose Unicode name contains the substring
    /// (whitelist by name). When non-empty, a character must match at least one
    /// --include-name pattern (or be listed via --include) to survive.
    ///
    /// Matching is case-insensitive substring matching:
    ///
    ///   --include-name katakana          # keep all katakana
    ///   --include-name hiragana          # keep all hiragana
    ///   --include-name "cjk unified ideograph"  # keep joyo kanji range (if only...)
    ///   --include-name katakana --include-name hiragana  # keep both
    ///
    /// Composes with --filter-name: blacklisted names are still excluded even
    /// when they would match an include-name pattern. Works for etlcdb and
    /// kanjivg.
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    include_name: Vec<String>,

    /// Expand output npz image files from (N,H,W) to (N,3,H,W) by computing
    /// three preprocessing channels for every image in parallel.
    ///
    /// Channel layout (C row-major, consistent with PyTorch CHW convention):
    ///   0 - original grayscale (raw copy)
    ///   1 - Otsu binary          (bright -> 255, invert=false)
    ///   2 - Sauvola binary       (w=11, k=0.20, bright -> 255)
    ///
    /// All three views are computed and stored so training never has to
    /// recompute preprocessing - the GPU stays fed. No selection heuristic
    /// is applied; the CNN learns what each channel means.
    ///
    /// Processing is Rayon-parallel. Both train and test halves are expanded
    /// when `--train-split` > 0. Opt-in; existing convert runs are unaffected.
    #[arg(long, default_value_t = false)]
    three_channel: bool,

    /// Merge halfwidth katakana into their standard fullwidth forms.
    ///
    /// When enabled (default), halfwidth katakana (U+FF65..U+FF9F) from
    /// M-type and C-type ETL records are mapped to their fullwidth katakana
    /// equivalents (U+30A1..U+30FC) so they share class labels with the
    /// fullwidth katakana from B/G-type records.
    ///
    /// U+FF9E and U+FF9F (halfwidth voiced/semi-voiced sound marks) map to
    /// U+309B/U+309C; those are normally caught by the "sound mark" filter
    /// before they reach the equiv step.
    ///
    /// Merging happens AFTER filtering, so excluded characters are removed
    /// first, then remaining halfwidth chars are merged into their canonical
    /// fullwidth form.
    #[arg(long, default_value_t = true)]
    merge_halfwidth: bool,
}

/// Expand a single (N,H,W) image npz to (N,3,H,W) in-place.
///
/// For every image three channels are computed in parallel with Rayon and
/// written back to the same path:
///   channel 0 - original grayscale
///   channel 1 - Otsu binary
///   channel 2 - Sauvola binary (w=11, k=0.20)
///
/// The output uses C row-major order so the flat index of pixel (r,c) in
/// channel `ch` of image `i` is `i*3*H*W + ch*H*W + r*W + c`.
///
/// Memory layout: allocates a single pre-sized output buffer `out` of exactly
/// `N*3*H*W` bytes, then fills each image's three channel slots in parallel
/// via Rayon - there is no intermediate `Vec<[Vec<u8>; 3]>` accumulator, so
/// peak RAM is approximately `raw_npz + out` (≈ 2x single-channel size rather
/// than the previous ≈ 5x).
#[cfg(not(target_arch = "wasm32"))]
fn expand_to_three_channels_npz(path: &str) -> Result<(), String> {
    use rayon::prelude::*;

    let raw = lib::npz::read_npz_first_entry(path)?;
    let hdr = lib::npz::parse_npy_header(&raw, path)?;

    if hdr.shape.len() != 3 {
        return Err(format!(
            "{path}: expected 3-D array (N,H,W), got shape {:?}",
            hdr.shape
        ));
    }
    let (n, h, w) = (hdr.shape[0], hdr.shape[1], hdr.shape[2]);
    let ppi = h * w;

    // Copy the pixel slice out of `raw` so we can drop the compressed NPZ
    // bytes before allocating the (3x larger) output buffer.
    let pixels: Vec<u8> = raw[hdr.data_offset..].to_vec();
    drop(raw); // release the full decompressed NPZ - no longer needed

    // Pre-allocate the exact-sized output buffer.
    // Layout: image-major, then channel-major inside each image so that
    // the flat index of pixel (r,c) in channel `ch` of image `i` is
    //   i*3*ppi + ch*ppi + r*w + c
    let mut out = vec![0u8; n * 3 * ppi];

    // Fill every image's three-channel block in parallel. Each image `i`
    // writes into a non-overlapping `3*ppi`-byte slice of `out`.
    out.par_chunks_mut(3 * ppi)
        .enumerate()
        .for_each(|(i, slot)| {
            let ch = lib::img::compute_three_channels(&pixels[i * ppi..(i + 1) * ppi], w, h);
            slot[..ppi].copy_from_slice(&ch[0]);
            slot[ppi..2 * ppi].copy_from_slice(&ch[1]);
            slot[2 * ppi..3 * ppi].copy_from_slice(&ch[2]);
        });

    tracing::info!(path, images = n, h, w, "expanded to 3-channel (N,3,H,W)");
    lib::npz::write_npz(path, &out, &[n, 3, h, w], "|u1")
        .map_err(|e| format!("failed to write 3-channel npz to {path}: {e}"))
}

/// Expand all output image npz files to 3 channels.
///
/// Processes `paths.imgs` always; also processes `paths.test_imgs` when
/// present (i.e. when `--train-split` > 0, both halves are expanded).
#[cfg(not(target_arch = "wasm32"))]
fn expand_outputs_to_three_channels(paths: &crate::etl::OutputPaths) {
    let mut targets: Vec<&str> = vec![&paths.imgs];
    if let Some(ref t) = paths.test_imgs {
        targets.push(t.as_str());
    }
    for path in targets {
        tracing::info!(path, "expanding to 3-channel (original + Otsu + Sauvola)");
        if let Err(e) = expand_to_three_channels_npz(path) {
            tracing::warn!(path, error = %e, "3-channel expansion failed - skipping");
        }
    }
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

    // Build merged filter and include lists: config base + CLI args additive
    //
    // If --filter-config is provided it is loaded first and its chars/names
    // form the starting set. Any --filter / --filter-name / --include /
    // --include-name values on the command line are then appended so the two
    // sources compose freely.
    // The merge_halfwidth flag defaults to true, but can be overridden by
    // --filter-config or CLI args.
    let (filter_chars, filter_names, include_chars, include_names, merge_hw) = {
        let mut merge_hw = args.merge_halfwidth;

        let (mut cfg_chars, mut cfg_names, mut cfg_include_chars, mut cfg_include_names): (
            Vec<char>,
            Vec<String>,
            Vec<char>,
            Vec<String>,
        ) = if let Some(ref path) = args.filter_config {
            let cfg = FilterConfig::from_file(path).unwrap_or_else(|e| {
                eprintln!("--filter-config: {e}");
                std::process::exit(1);
            });
            tracing::info!(
                path,
                chars = cfg.chars.len(),
                names = cfg.names.len(),
                include_chars = cfg.include_chars.len(),
                include_names = cfg.include_names.len(),
                merge_hw = cfg.merge_halfwidth,
                "loaded filter config"
            );
            // Config file value is used as default, CLI args override
            merge_hw = cfg.merge_halfwidth;
            (
                cfg.parse_chars(),
                cfg.names_lowercased(),
                cfg.parse_include_chars(),
                cfg.include_names_lowercased(),
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };

        // Append CLI --filter chars deduplicated.
        for ch in parse_chars(&args.filter) {
            if !cfg_chars.contains(&ch) {
                cfg_chars.push(ch);
            }
        }

        // Append CLI --filter-name substrings deduplicated and lowercased.
        for name in args.filter_name.iter().map(|s| s.to_lowercase()) {
            if !cfg_names.contains(&name) {
                cfg_names.push(name);
            }
        }

        // Append CLI --include chars deduplicated.
        for ch in parse_chars(&args.include) {
            if !cfg_include_chars.contains(&ch) {
                cfg_include_chars.push(ch);
            }
        }

        // Append CLI --include-name substrings deduplicated and lowercased.
        for name in args.include_name.iter().map(|s| s.to_lowercase()) {
            if !cfg_include_names.contains(&name) {
                cfg_include_names.push(name);
            }
        }

        (
            cfg_chars,
            cfg_names,
            cfg_include_chars,
            cfg_include_names,
            merge_hw,
        )
    };

    // Build the equiv map once from the resolved merge flag. Every writer and
    // label-map builder receives this map; an empty map means no merging.
    let equiv = if merge_hw {
        crate::kana_merging::halfwidth_equiv()
    } else {
        std::collections::HashMap::new()
    };

    if args.format.to_lowercase() == "kanjivg" {
        kanjivg::convert_kanjivg_dir(
            &args.input,
            &paths,
            args.train_split,
            args.seed,
            args.aug_seed,
            &filter_chars,
            &filter_names,
            &include_chars,
            &include_names,
            &equiv,
        )
        .unwrap_or_else(|e| {
            eprintln!("kanjivg conversion failed: {e}");
            std::process::exit(1);
        });
        if args.three_channel {
            expand_outputs_to_three_channels(&paths);
        }
        return;
    }

    // Combined ETL mode: reads ETL1/6/7/8B/9B/8G/9G from one root dir and
    // writes per-family npz files with a shared classmap + stats.
    if args.format.to_lowercase() == "etlcdb" {
        // Convert Vec<String> to Vec<&str> for read_etlcdb_dir
        let exclude: Vec<&str> = args.exclude.iter().map(|s| s.as_str()).collect();

        if !exclude.is_empty() {
            tracing::info!(exclude = ?exclude, "excluding datasets");
        }

        tracing::info!(root = %args.input, out = %args.out, "converting all ETL datasets");
        let batches = read_etlcdb_dir(&args.input, &exclude).unwrap_or_else(|e| {
            eprintln!("failed to read ETLcdb from {}: {e}", args.input);
            std::process::exit(1);
        });
        let total: usize = batches.iter().map(|b| b.records.len()).sum();
        tracing::info!(
            total_records = total,
            batches = batches.len(),
            "ETLcdb read complete"
        );
        if args.merge {
            let (dst_w, dst_h) = max_batch_dims(&batches);
            tracing::info!(
                dst_w,
                dst_h,
                "merging all batches to square power-of-two resolution"
            );
            write_merged_etlcdb(
                &batches,
                &args.out,
                args.train_split,
                dst_w,
                dst_h,
                &filter_chars,
                &filter_names,
                &include_chars,
                &include_names,
                &equiv,
            )
            .unwrap_or_else(|e| {
                eprintln!("ETLcdb merged conversion failed: {e}");
                std::process::exit(1);
            });
        } else {
            convert_etlcdb(
                &batches,
                &args.out,
                args.train_split,
                &filter_chars,
                &filter_names,
                &include_chars,
                &include_names,
                &equiv,
            )
            .unwrap_or_else(|e| {
                eprintln!("ETLcdb conversion failed: {e}");
                std::process::exit(1);
            });
        }
        if args.three_channel {
            expand_outputs_to_three_channels(&paths);
        }
        return;
    }

    // TODO: Future mitch this is yeetable out of the entire code base unless
    // ETL is eventually useful?
    let format = EtlFormat::from_str(&args.format).unwrap_or_else(|| {
        eprintln!(
            "unknown format {:?}; valid values: etl1, etl6, etl7, etl8b, etl9b, etl8g, etl9g, etlcdb",
            args.format
        );
        std::process::exit(1);
    });

    let input_path = std::path::Path::new(&args.input);
    // Single-file/directory mode doesn't support --exclude, pass empty slice
    let records = if input_path.is_dir() {
        read_etl_dir(&args.input, format, &[])
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
        let map = label_map_from_records(&records, &equiv);
        tracing::info!(classes = map.len(), "auto-detected label map from records");
        map
    };

    convert_to_npz(&records, &paths, &label_map, args.train_split, &equiv).unwrap_or_else(|e| {
        eprintln!("failed to write npz: {e}");
        std::process::exit(1);
    });
}
