//! KanjiVG SVG -> 28x28 npz training dataset conversion.
//!
//! KanjiVG (<https://kanjivg.tagaini.net>) provides clean bezier-curve stroke
//! data for 6,355+ kanji and kana on a 109x109 canvas. Each character has a
//! file named by its Unicode code point: `04e00.svg` = 一.
//!
//! ## Augmentation strategy
//!
//! A single SVG gives one canonical drawing. We synthesize `N_AUG` training
//! samples per stroke width variant by rendering once then applying random
//! geometric augmentation rotation +/-15deg, translation +/-2px, scale 0.85-1.15
//
//TODO: This synthetic data is incomplete and needs augmentation. That is a
// future me task.
use std::{collections::HashMap, io, sync::LazyLock};

use rand::SeedableRng;
use rand::seq::SliceRandom;
use regex::Regex;

use crate::data::augment_image;
use crate::etl::{OutputPaths, write_classmap, write_npz, write_stats};

// KanjiVG images are always rendered and stored at 28x28 pixels.
const KANJI_IMG_SIZE: usize = 28;

use crate::etl::{FilterReason, filter_reason};

/// Stroke widths in KanjiVG's 109-unit coordinate space to render. The default
/// in the SVG files is 3. Vary it to simulate different pen pressures and brush
/// sizes even though the input is pure black in the egui setup.
//const STROKE_WIDTHS: [f32; 4] = [2.5, 3.0, 3.5, 4.5];
const STROKE_WIDTHS: [f32; 3] = [2.5, 3.0, 3.5];

/// Total samples generated per stroke width 1 clean; rest augmented.
const N_AUG: usize = 100;

/// KanjiVG canvas size in SVG units.
const KVG_SIZE: f32 = 109.0;

/// Output image size in pixels to match mnist for now. Need to find out if I
/// should bother keeping 28x28 for this or not.
const OUT_SIZE: u32 = 28;

/// Convert a directory of KanjiVG svg files to paired npz training files.
///
/// Reads every `*.svg` file in `dir`, skips variant forms which are just
/// filenames containing a hyphen before `.svg` and aren't useful to train
/// against, renders each character at `STROKE_WIDTHS` x `N_AUG` augmented
/// samples, and writes the result to the paths derived from `paths` from the
/// cli.
#[allow(clippy::too_many_arguments)]
pub fn convert_kanjivg_dir(
    dir: &str,
    paths: &OutputPaths,
    train_fraction: f64,
    seed: Option<u64>,
    aug_seed: Option<u64>,
    filter_chars: &[char],
    filter_names: &[String],
    include_chars: &[char],
    include_names: &[String],
    equiv: &std::collections::HashMap<char, char>,
) -> io::Result<()> {
    let mut entries: Vec<(char, std::path::PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let stem = path.file_stem()?.to_str()?.to_owned();
            let ext = path.extension()?.to_str()?;
            if ext != "svg" {
                return None;
            }
            // Skip variant forms too e.g. 04e00-Kaisho.svg they just make things weird
            if stem.contains('-') {
                return None;
            }
            let ch = char_from_hex(&stem)?;
            Some((ch, path))
        })
        .collect();

    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no base KanjiVG svg files found in {dir}"),
        ));
    }

    entries.sort_by_key(|(ch, _)| *ch as u32);
    tracing::info!(dir, chars = entries.len(), "KanjiVG files found");

    // Apply --filter / --filter-name aka blacklist and --include / --include-name
    // whitelist exclusions before building the label map so that class indices
    // remain contiguous without gaps.
    let has_filter = !filter_chars.is_empty() || !filter_names.is_empty();
    let has_include = !include_chars.is_empty() || !include_names.is_empty();
    if has_filter || has_include {
        if !filter_chars.is_empty() {
            let chars_repr: Vec<String> = filter_chars
                .iter()
                .map(|c| format!("{c:?} U+{:04X}", *c as u32))
                .collect();
            tracing::info!(patterns = %chars_repr.join(", "), "--filter is filtering");
        }
        if !filter_names.is_empty() {
            tracing::info!(patterns = %filter_names.join(", "), "--filter-name is filtering");
        }
        if !include_chars.is_empty() {
            let chars_repr: Vec<String> = include_chars
                .iter()
                .map(|c| format!("{c:?} U+{:04X}", *c as u32))
                .collect();
            tracing::info!(patterns = %chars_repr.join(", "), "--include is whitelisting");
        }
        if !include_names.is_empty() {
            tracing::info!(patterns = %include_names.join(", "), "--include-name is whitelisting");
        }

        let mut removed_by_char: usize = 0;
        let mut removed_by_name: usize = 0;
        let mut removed_by_whitelist: usize = 0;

        entries.retain(|(ch, _)| {
            match filter_reason(
                *ch,
                filter_chars,
                filter_names,
                include_chars,
                include_names,
            ) {
                None => true,
                Some(FilterReason::ExplicitChar) => {
                    tracing::debug!(
                        char = %ch,
                        codepoint = format!("U+{:04X}", *ch as u32),
                        "excluded by --filter"
                    );
                    removed_by_char += 1;
                    false
                }
                Some(FilterReason::NameSubstring {
                    ref pattern,
                    ref name,
                }) => {
                    tracing::debug!(
                        char = %ch,
                        codepoint = format!("U+{:04X}", *ch as u32),
                        unicode_name = %name,
                        matched_pattern = %pattern,
                        "excluded by --filter-name"
                    );
                    removed_by_name += 1;
                    false
                }
                Some(FilterReason::NotInWhitelist { ref name }) => {
                    tracing::debug!(
                        char = %ch,
                        codepoint = format!("U+{:04X}", *ch as u32),
                        unicode_name = %name,
                        "excluded: not in --include / --include-name whitelist"
                    );
                    removed_by_whitelist += 1;
                    false
                }
            }
        });

        let removed = removed_by_char + removed_by_name + removed_by_whitelist;
        if removed > 0 {
            tracing::info!(
                removed,
                removed_by_char,
                removed_by_name,
                removed_by_whitelist,
                remaining = entries.len(),
                "characters filtered out"
            );
        } else {
            tracing::warn!(
                "filter/include flags were provided however nothing was removed either the inputs aren't in the dataset or the patterns are wrong. Re-run with RUST_LOG=debug for more details"
            );
        }
    }

    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "all characters were filtered out and there is nothing to convert? seems sus",
        ));
    }

    // Build label map directly from filtered entries, normalizing through equiv
    // at lookup time - no need to mutate or clone the entries list.
    // entries is already sorted by char (unicode point order).
    let canonical_chars: Vec<char> = entries
        .iter()
        .map(|(ch, _)| crate::kana_merging::equiv_char(*ch, equiv))
        .collect();
    let label_map: HashMap<char, u32> = canonical_chars
        .iter()
        .enumerate()
        .map(|(i, &ch)| (ch, i as u32))
        .collect();

    let n_classes = entries.len();
    let samples_per_char = STROKE_WIDTHS.len() * N_AUG;
    let _n_samples = n_classes * samples_per_char;

    // When a train/test split is requested we split *per character* so that
    // every class appears in both the train and test sets. A flat sequential
    // split would put the last ~20% of characters exclusively in test, making
    // validation accuracy meaningless as the model has never seen those classes
    // yet. Ask me how I know. While this new approach is better its still not
    // great.
    let doing_split = train_fraction > 0.0;
    let train_per_char = if doing_split {
        ((samples_per_char as f64 * train_fraction).floor() as usize).max(1)
    } else {
        samples_per_char
    };
    let test_per_char = samples_per_char - train_per_char;

    let mut train_images: Vec<u8> = Vec::with_capacity(n_classes * train_per_char * 784);
    let mut train_labels: Vec<u32> = Vec::with_capacity(n_classes * train_per_char);
    let mut test_images: Vec<u8> = Vec::with_capacity(n_classes * test_per_char * 784);
    let mut test_labels: Vec<u32> = Vec::with_capacity(n_classes * test_per_char);

    // Augmentation RNG - seeded from --aug-seed when provided for fully
    // reproducible geometric perturbations; otherwise non-deterministic so
    // each convert run produces different rotations/translations/scales.
    let mut aug_rng: Box<dyn rand::Rng> = match aug_seed {
        Some(s) => {
            tracing::info!(
                aug_seed = s,
                "using seeded RNG for augmentation perturbations"
            );
            Box::new(rand::rngs::SmallRng::seed_from_u64(s))
        }
        None => Box::new(rand::rng()),
    };

    // Split RNG - only created when --seed is given. A separate RNG keeps the
    // split selection fully independent from the augmentation decisions, so
    // changing --seed never alters the rendered pixels, only which samples land
    // in train vs test.
    let mut split_rng: Option<rand::rngs::SmallRng> = seed.map(rand::rngs::SmallRng::seed_from_u64);

    if let Some(s) = seed {
        tracing::info!(
            seed = s,
            "using seeded RNG for per-character sample selection"
        );
    }

    // Track how many svgs have failed to render; only panic after this
    // threshold so we can inspect multiple failures without aborting early. Not
    // entirely sure why some widths fail yet and this hack gets me data I can
    // train on to test.
    let mut render_fail_count: usize = 0;
    const PANIC_AFTER_FAILS: usize = 4;

    for (idx, (ch, path)) in entries.iter().enumerate() {
        if idx % 500 == 0 {
            tracing::info!(
                "{}/{} ({:.0}%) - processing {}",
                idx,
                n_classes,
                idx as f32 / n_classes as f32 * 100.0,
                ch
            );
        }

        let svg_raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("skipping {}: {e}", path.display());
                continue;
            }
        };

        let svg_stripped = strip_namespaces(&svg_raw);
        let svg_clean = strip_stroke_numbers(&svg_stripped);
        let canonical = crate::kana_merging::equiv_char(*ch, equiv);
        let label = match label_map.get(&canonical) {
            Some(&l) => l,
            None => {
                tracing::warn!(
                    "skipping U+{:X} '{}': canonical U+{:X} '{}' not in label_map",
                    *ch as u32,
                    ch,
                    canonical as u32,
                    canonical
                );
                continue;
            }
        };

        // Collect all samples for this character into a temporary per-char
        // buffer, then route them into train/test based on position.
        let mut char_images: Vec<[u8; 784]> = Vec::with_capacity(samples_per_char);

        for &sw in &STROKE_WIDTHS {
            let svg = set_stroke_width(&svg_clean, sw);

            let base_px = match render_to_28x28(&svg) {
                Some(px) => px,
                None => {
                    // Dump the cleaned svg to a temp file for inspection in the case of too many failures
                    let dump_name = format!("kanjivg_failed_U+{:X}_sw{:.1}.svg", *ch as u32, sw);
                    let dump_path = std::env::temp_dir().join(&dump_name);
                    match std::fs::write(&dump_path, &svg) {
                        Ok(()) => {
                            tracing::error!(path=%dump_path.display(), "dumped cleaned svg for failed parse")
                        }
                        Err(e) => {
                            tracing::error!(?e, "failed to write dumped svg to {:?}", dump_path)
                        }
                    }

                    // Increment failure counter and only panic after a threshold is reached.
                    render_fail_count += 1;
                    if render_fail_count >= PANIC_AFTER_FAILS {
                        panic!(
                            "KanjiVG render failed for {} (stroke-width={sw}). Dumped cleaned svg to {}; aborting after {} failures",
                            ch,
                            dump_path.display(),
                            render_fail_count
                        );
                    }

                    // Otherwise, skip this render and continue processing like nothing happened like a hack
                    tracing::warn!(
                        "render failed for {} (stroke-width={sw}); dumped to {} (failure {}/{})",
                        ch,
                        dump_path.display(),
                        render_fail_count,
                        PANIC_AFTER_FAILS
                    );
                    continue;
                }
            };

            char_images.push(base_px);

            // augment_image now takes &[u8] directly - no u8->f32->u8 round-trip.
            for _ in 1..N_AUG {
                let aug = augment_image(&base_px, 28, 28, &mut *aug_rng);
                char_images.push(f32_to_u8_image(&aug));
            }
        }

        // When a seed is supplied, shuffle the per-character samples so the
        // clean render and augmented variants are randomly distributed between
        // train and test rather than always putting the clean render in train.
        // Note: this randomness is deterministic.
        if let Some(ref mut srng) = split_rng {
            char_images.shuffle(srng);
        }

        for (i, px) in char_images.iter().enumerate() {
            if i < train_per_char {
                train_images.extend_from_slice(px);
                train_labels.push(label);
            } else {
                test_images.extend_from_slice(px);
                test_labels.push(label);
            }
        }
    }

    let n_train = train_labels.len();
    let n_test = test_labels.len();
    let n = n_train + n_test;
    tracing::info!(
        n,
        n_train,
        n_test,
        classes = n_classes,
        "rendering complete"
    );

    let label_dtype = "<u2";

    let out_dir = std::path::Path::new(&paths.imgs)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");

    write_classmap(out_dir, &label_map)?;
    write_stats(out_dir, &train_images, KANJI_IMG_SIZE * KANJI_IMG_SIZE)?;

    if n == 0 {
        return Err(io::Error::other("no images were successfully rendered"));
    }

    let serialize_labels = |labels: &[u32]| -> Vec<u8> {
        labels
            .iter()
            .flat_map(|&l| (l as u16).to_le_bytes())
            .collect()
    };

    match (&paths.test_imgs, &paths.test_labels) {
        (Some(test_imgs_path), Some(test_labels_path)) => {
            let train_label_bytes = serialize_labels(&train_labels);
            let test_label_bytes = serialize_labels(&test_labels);

            write_npz(&paths.imgs, &train_images, &[n_train, 28, 28], "|u1")?;
            write_npz(&paths.labels, &train_label_bytes, &[n_train], label_dtype)?;
            write_npz(test_imgs_path, &test_images, &[n_test, 28, 28], "|u1")?;
            write_npz(test_labels_path, &test_label_bytes, &[n_test], label_dtype)?;

            tracing::info!(
                train = n_train,
                test  = n_test,
                train_per_char,
                test_per_char,
                imgs_out   = %paths.imgs,
                labels_out = %paths.labels,
                "kanjivg per-character split written"
            );
        }
        _ => {
            let all_images: Vec<u8> = train_images
                .iter()
                .chain(test_images.iter())
                .copied()
                .collect();
            let all_labels: Vec<u32> = train_labels
                .iter()
                .chain(test_labels.iter())
                .copied()
                .collect();
            let all_label_bytes = serialize_labels(&all_labels);

            write_npz(&paths.imgs, &all_images, &[n, 28, 28], "|u1")?;
            write_npz(&paths.labels, &all_label_bytes, &[n], label_dtype)?;
            tracing::info!(n, imgs_out = %paths.imgs, "kanjivg written");
        }
    }

    Ok(())
}

/// Remove the `<g id="kvg:StrokeNumbers_...">` stroke subtree from a KanjiVG svg.
///
/// This will be useful for the ui in learning kanji stroke order but for
/// detection its ass.
///
/// This is a huge af hack written in anger.
fn strip_stroke_numbers(svg: &str) -> String {
    let marker = "<g id=\"kvg:StrokeNumbers";
    let Some(start) = svg.find(marker) else {
        return svg.to_owned();
    };

    // Walk forward from `start`, counting <g ... > / </g> nesting depth so we
    // find the correct matching </g> even for groups that contain sub-groups.
    let tail = &svg[start..];
    let mut depth = 0usize;
    let mut pos = 0usize;

    while pos < tail.len() {
        if tail[pos..].starts_with("</g>") {
            if depth == 1 {
                let end = start + pos + 4;
                return format!("{}{}", &svg[..start], &svg[end..]);
            }
            depth = depth.saturating_sub(1);
            pos += 4;
        } else if tail[pos..].starts_with("<g") {
            depth += 1;
            pos += 2;
        } else {
            pos += 1;
        }
    }

    // Couldn't find matching </g> - return unchanged.
    svg.to_owned()
}

// Static regexes compiled once for strip_namespaces avoids per-call
// allocation and removes all Regex::new().unwrap() call sites.
static RE_XML_PROLOG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<\?xml[^>]*\?>").expect("static regex is valid"));
static RE_DOCTYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!DOCTYPE[^>]*?(?:\[[\s\S]*?\])?\s*>").expect("static regex is valid")
});
static RE_KVG_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\s+kvg:[\w-]+=(?:"[^"]*"|'[^']*')"#).expect("static regex is valid")
});
static RE_XMLNS_PREF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\s+xmlns:[A-Za-z_][\w-]*=(?:"[^"]*"|'[^']*')"#).expect("static regex is valid")
});
static RE_NS_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\s+[A-Za-z_][\w-]*:[\w-]+=(?:"[^"]*"|'[^']*')"#).expect("static regex is valid")
});

/// Strip namespace-related declarations and attributes from KanjiVG svg so we
/// can render it with resvg. Which hates the namespace crap in it.
///
/// Could really use a unit test but whatever.
fn strip_namespaces(svg: &str) -> String {
    // Remove XML prolog (<?xml ... ?>)
    let mut s = RE_XML_PROLOG.replace_all(svg, "").to_string();

    // Remove DOCTYPE including optional internal subset and trailing '>'
    // This matches: <!DOCTYPE ...> and <!DOCTYPE ... [ ... ]>
    s = RE_DOCTYPE.replace_all(&s, "").to_string();

    // Remove kvg: namespaced attributes aka kvg:element="..."
    s = RE_KVG_ATTR.replace_all(&s, "").to_string();

    // Remove xmlns:prefix declarations but preserve default xmlns="..."
    s = RE_XMLNS_PREF.replace_all(&s, "").to_string();

    // Remove any other attributes with a namespace prefix aka e.g. foo:bar='...'
    s = RE_NS_ATTR.replace_all(&s, "").to_string();

    // Ensure the string starts at the <svg ...> root element. Some files include
    // comments or other headers before the svg; `usvg` requires a root node.
    if let Some(pos) = s.find("<svg") {
        s = s[pos..].to_string();
    }

    s
}

/// Replace the stroke width in a KanjiVG svg string.
fn set_stroke_width(svg: &str, width: f32) -> String {
    // KanjiVG always uses exactly "stroke-width:3"  this hack replace regex seems fine.
    svg.replace("stroke-width:3", &format!("stroke-width:{width:.1}"))
}

/// Render a KanjiVG svg string to a 28x28 grayscale image.
///
/// Returns `None` if the svg fails to parse or the pixmap cannot be created.
///
/// **Pixel convention (matches training pipeline):**
/// - `0`   = background (white)
/// - `255` = ink (black stroke, rendered as bright = high value)
fn render_to_28x28(svg: &str) -> Option<[u8; 784]> {
    let opt = resvg::usvg::Options::default();
    let render = resvg::usvg::Tree::from_str(svg, &opt);
    if let Err(e) = render {
        eprintln!("{:?}", e);
        return None;
    }
    let tree = render.ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(OUT_SIZE, OUT_SIZE)?;
    // White background - the svg strokes are black, so background = white for the CNN.
    pixmap.fill(resvg::tiny_skia::Color::WHITE);

    let scale = OUT_SIZE as f32 / KVG_SIZE;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Convert RGBA -> grayscale; then invert so ink = 255, background = 0.
    let rgba = pixmap.data(); // [R, G, B, A, R, G, B, A, ...]
    let mut out = [0u8; 784];
    for (i, chunk) in rgba.chunks_exact(4).enumerate().take(784) {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        let gray = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
        out[i] = 255 - gray; // invert: black ink -> 255 (bright), white bg -> 0
    }
    Some(out)
}

/// Convert a flat `f32` pixel buffer back to `[u8; 784]`, clamping to 0–255.
fn f32_to_u8_image(img: &[f32]) -> [u8; 784] {
    let mut out = [0u8; 784];
    for (i, &v) in img.iter().enumerate().take(784) {
        out[i] = v.clamp(0.0, 255.0) as u8;
    }
    out
}

/// Parse a KanjiVG file stem aka `"04e00"` to its Unicode character code.
///
/// Returns `None` for variant forms or code points that don't map to a valid
/// unicode `char`.
fn char_from_hex(stem: &str) -> Option<char> {
    if stem.contains('-') {
        return None; // variant form - skip
    }
    let code = u32::from_str_radix(stem, 16).ok()?;
    char::from_u32(code)
}
