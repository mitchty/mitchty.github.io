// Snapshot and SSIM helpers for shader image output tests.
//
// # Fixture paths
//
// `assert_snapshot` accepts a `&[&str]` path slice rather than a flat name
// string. All elements except the last are subdirectory components under
// `tests/fixtures/`; the last element is the PNG stem. This keeps the fixture
// directory from becoming a flat pile of crap and lets refactors change the code path
// independently of the snapshot files or not.
//
// Example:
//   assert_snapshot(&["plot", "canonical"], &frame, threshold, None)
//   -> tests/fixtures/plot/canonical.png
//
// # Canonical-only fixture model
//
// Only the canonical render is saved as a fixture png. Every other shader
// variant is validated with `assert_variant_matches_canonical`, which loads the
// canonical PNG from disk and compares the variant frame against it. On failure
// a diff PNG is written to the same directory as the canonical and its path is
// included in the panic message.
//
// # PNG size guard
//
// Both `assert_snapshot` and `assert_variant_matches_canonical` accept an
// optional `size_ratio: Option<f64>` parameter. Pass `None` to use the
// default constant `PNG_SIZE_RATIO_MIN`, or `Some(ratio)` to override per
// call-site.
//
// The guard encodes both images to PNG bytes in memory and checks that
// `min_bytes / max_bytes >= ratio`. A blank 256x256 frame compresses to
// ~1.9 KB while a frame with real content compresses to ~5.7 KB (ratio ≈ 0.33),
// so the default 0.50 threshold reliably catches blank-vs-real comparisons
// that would otherwise pass SSIM because both sides are blank.

use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use std::path::PathBuf;

use super::bevy::RenderedFrame;

/// SSIM threshold below which a snapshot comparison is considered failed.
/// 0.95 = allow up to 5% structural difference between different shaders/gpus.
pub const DEFAULT_SSIM_THRESHOLD: f64 = 0.95;

/// Minimum ratio smaller / larger of PNG-compressed byte sizes.
///
/// If `min(rendered_bytes, reference_bytes) / max(rendered_bytes, reference_bytes)`
/// falls below this value the test fails before SSIM is even evaluated.
///
/// Callers can tighten or loosen the 50% by default per call-site by passing
/// `Some(your_value)` to `assert_snapshot` or
/// `assert_variant_matches_canonical` if the 50% is off. I had to pick a number
/// and this is good enough to catch the issues I was seeing.
pub const PNG_SIZE_RATIO_MIN: f64 = 0.50;

/// Encode an `RgbaImage` to PNG bytes in memory.
fn encode_png_bytes(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .expect("in-memory png encode cannot fail");
    buf
}

/// Resolve a path slice to an absolute file path.
///
/// `["plot", "wgsl", "canonical"]` -> `<manifest>/tests/fixtures/plot/wgsl/canonical.png`
///
/// Panics if `path` is empty.
fn fixture_path(path: &[&str]) -> PathBuf {
    assert!(
        !path.is_empty(),
        "snapshot path must have at least one component"
    );
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let mut p = PathBuf::from(manifest).join("tests").join("fixtures");
    for component in &path[..path.len() - 1] {
        p = p.join(component);
    }
    p.join(format!("{}.png", path[path.len() - 1]))
}

/// Per-pixel red-intensity difference image, amplified 8x for visibility.
fn diff_image(rendered: &RgbaImage, reference: &RgbaImage) -> RgbaImage {
    let (w, h) = (rendered.width(), rendered.height());
    let mut diff = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let r = rendered.get_pixel(x, y);
            let e = reference.get_pixel(x, y);
            let dr = (r[0] as i16 - e[0] as i16).unsigned_abs() as u32;
            let dg = (r[1] as i16 - e[1] as i16).unsigned_abs() as u32;
            let db = (r[2] as i16 - e[2] as i16).unsigned_abs() as u32;
            let mag = dr.max(dg).max(db).saturating_mul(8).min(255) as u8;
            diff.put_pixel(x, y, Rgba([mag, 0, 0, 255]));
        }
    }
    diff
}

/// Three-panel side-by-side composite when the image differs: expected | found | red-diff.
fn composite_diff(reference: &RgbaImage, rendered: &RgbaImage, diff: &RgbaImage) -> RgbaImage {
    let (w, h) = (reference.width(), reference.height());
    const SEP: u32 = 2;
    let total_w = w * 3 + SEP * 2;
    let mut out = RgbaImage::new(total_w, h);
    for pixel in out.pixels_mut() {
        *pixel = Rgba([255, 255, 255, 255]);
    }
    for (src, x_off) in [(reference, 0), (rendered, w + SEP), (diff, w * 2 + SEP * 2)] {
        for y in 0..h {
            for x in 0..w {
                out.put_pixel(x + x_off, y, *src.get_pixel(x, y));
            }
        }
    }
    out
}

/// Check the png byte size ratio and panic with a message if it is too far off.
///
/// `label` is in the panic(). `ratio_override` takes precedence over
/// `PNG_SIZE_RATIO_MIN` when `Some`.
fn check_png_size_ratio(label: &str, a_bytes: &[u8], b_bytes: &[u8], ratio_override: Option<f64>) {
    let ratio_min = ratio_override.unwrap_or(PNG_SIZE_RATIO_MIN);
    let a = a_bytes.len();
    let b = b_bytes.len();
    let (smaller, larger) = if a <= b { (a, b) } else { (b, a) };
    // Avoid division by zero on degenerate empty images that somehow got written.
    if larger == 0 {
        return;
    }
    let ratio = smaller as f64 / larger as f64;
    if ratio < ratio_min {
        panic!(
            "{label}: png size ratio {ratio:.3} is below minimum {ratio_min:.3}\n\
             sizes are {a} bytes vs {b} bytes\n\
             If this is intentional aka a legit sparse output render, change \
             Some(lower_ratio) to the assertion and rerun.",
        );
    }
}

/// Convert a `RenderedFrame` to an `RgbaImage`.
pub fn frame_to_image(frame: &RenderedFrame) -> RgbaImage {
    ImageBuffer::from_raw(frame.width, frame.height, frame.pixels.clone())
        .expect("pixel buffer size does not match declared dimensions")
}

/// Return `true` if the frame contains at least one pixel that differs from
/// the clear color transparent black.
///
/// The render harness clears to `TRANSPARENT (0,0,0,0)`. Any pixel written
/// by a shader will have `alpha > 0`. A completely clear frame is certainly
/// a blank render and should fail any visual test regardless of SSIM score as
/// two blank frames compare at SSIM 1.0 against each other.
pub fn has_non_background_pixels(frame: &RenderedFrame) -> bool {
    frame.pixels.chunks_exact(4).any(|p| p[3] > 0)
}

/// Compare a rendered variant frame against the on-disk canonical fixture.
///
/// Loads the canonical PNG from `canonical_path`, then checks in order:
/// 1. Dimensions match.
/// 2. PNG size ratio `min/max >= size_ratio.unwrap_or(PNG_SIZE_RATIO_MIN)`.
/// 3. SSIM score `>= ssim_threshold`.
///
/// On failure a composite diff PNG is written to the **same directory** as the
/// canonical fixture, named `{variant_name}_diff.png`. The full path is
/// included in the panic message so it is immediately findable.
///
/// # Parameters
///
/// * `canonical_path` - path slice for the canonical fixture (same format as
///   `assert_snapshot`, e.g. `&["plot", "canonical"]`).
/// * `variant_name`   - short identifier for the variant under test, used as
///   the diff filename stem (e.g. `"plot_material"`).
/// * `actual`         - the rendered frame to validate.
/// * `ssim_threshold` - SSIM floor (e.g. `DEFAULT_SSIM_THRESHOLD`).
/// * `size_ratio`     - PNG size ratio floor; `None` uses `PNG_SIZE_RATIO_MIN`.
pub fn assert_variant_matches_canonical(
    canonical_path: &[&str],
    variant_name: &str,
    actual: &RenderedFrame,
    ssim_threshold: f64,
    size_ratio: Option<f64>,
) {
    let canon_png_path = fixture_path(canonical_path);
    let reference = image::open(&canon_png_path)
        .unwrap_or_else(|e| {
            panic!(
                "could not load canonical fixture {canon_png_path:?}: {e}\nRun with UPDATE_SNAPSHOTS=1 to create it."
            )
        })
        .to_rgba8();

    let actual_img = frame_to_image(actual);

    assert_eq!(
        (reference.width(), reference.height()),
        (actual_img.width(), actual_img.height()),
        "{variant_name}: dimension mismatch - canonical {}x{} vs actual {}x{}",
        reference.width(),
        reference.height(),
        actual_img.width(),
        actual_img.height(),
    );

    let reference_png = encode_png_bytes(&reference);
    let actual_png = encode_png_bytes(&actual_img);
    check_png_size_ratio(variant_name, &reference_png, &actual_png, size_ratio);

    let result = image_compare::rgba_hybrid_compare(&actual_img, &reference)
        .unwrap_or_else(|e| panic!("{variant_name}: ssim compare failed: {e}"));

    if result.score < ssim_threshold {
        let diff = diff_image(&actual_img, &reference);
        let composite = composite_diff(&reference, &actual_img, &diff);
        let diff_path = canon_png_path
            .parent()
            .expect("canonical fixture path must have a parent dir")
            .join(format!("{variant_name}_diff.png"));
        composite
            .save(&diff_path)
            .unwrap_or_else(|e| panic!("{variant_name}: could not save diff {diff_path:?}: {e}"));
        panic!(
            "{variant_name}: ssim {:.4} < threshold {:.4}\n\
             diff written to: {diff_path:?}\n\
             panels: canonical | actual | red-diff amplified 8x",
            result.score, ssim_threshold,
        );
    }
}

/// Compare a rendered frame against the existing fixture at `path`.
///
/// # Checks that:
///
/// 1. Dimensions match.
/// 2. PNG size ratio `min/max >= size_ratio.unwrap_or(PNG_SIZE_RATIO_MIN)`.
/// 3. SSIM score `>= ssim_threshold`.
///
/// # First run / UPDATE_SNAPSHOTS=1
///
/// The fixture is created/overwritten; no comparison is performed.
///
/// # Parameters
///
/// * `path`           - slice of path components
/// * `frame`          - the frame to compare or bless.
/// * `ssim_threshold` - SSIM floor  `DEFAULT_SSIM_THRESHOLD`.
/// * `size_ratio`     - PNG size ratio floor `None` uses `PNG_SIZE_RATIO_MIN`.
pub fn assert_snapshot(
    path: &[&str],
    frame: &RenderedFrame,
    ssim_threshold: f64,
    size_ratio: Option<f64>,
) {
    let png_path = fixture_path(path);
    let rendered = frame_to_image(frame);

    let update = std::env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !png_path.exists() || update {
        let dir = png_path
            .parent()
            .expect("fixture path must have a parent dir");
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|e| panic!("could not create fixture dir {dir:?}: {e}"));
        rendered
            .save(&png_path)
            .unwrap_or_else(|e| panic!("could not save snapshot {png_path:?}: {e}"));
        if update {
            println!("updated: {png_path:?}");
        } else {
            println!("created initial reference: {png_path:?}");
        }
        return;
    }

    let reference = image::open(&png_path)
        .unwrap_or_else(|e| panic!("could not load reference snapshot {png_path:?}: {e}"))
        .to_rgba8();

    assert_eq!(
        (rendered.width(), rendered.height()),
        (reference.width(), reference.height()),
        "snapshot {png_path:?}: dimension mismatch - rendered {}x{} vs reference {}x{}",
        rendered.width(),
        rendered.height(),
        reference.width(),
        reference.height(),
    );

    // PNG size check before SSIM.
    let label = path.join("/");
    let rendered_png = encode_png_bytes(&rendered);
    let reference_png = encode_png_bytes(&reference);
    check_png_size_ratio(&label, &rendered_png, &reference_png, size_ratio);

    let result = image_compare::rgba_hybrid_compare(&rendered, &reference)
        .unwrap_or_else(|e| panic!("ssim compare failed for {png_path:?}: {e}"));

    if result.score < ssim_threshold {
        let diff = diff_image(&rendered, &reference);
        let composite = composite_diff(&reference, &rendered, &diff);
        let stem = path[path.len() - 1];
        let diff_path = png_path
            .parent()
            .expect("png_path must have a parent directory")
            .join(format!("{stem}_diff.png"));
        composite
            .save(&diff_path)
            .unwrap_or_else(|e| panic!("could not save diff {diff_path:?}: {e}"));
        panic!(
            "snapshot {png_path:?}: ssim {:.4} < threshold {:.4}\n\
             diff saved to: {diff_path:?}\n\
             panels: expected | found | red-diff amplified 8x\n\
             rerun with UPDATE_SNAPSHOTS=1 to accept the new output",
            result.score, ssim_threshold,
        );
    }
}
