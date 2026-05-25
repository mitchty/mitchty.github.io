// Snapshot unit test helper functions and crap.
//
// Basically here to setup reference png images for fragment shaders to compare
// against. Whatever is in here is "blessed" or whatever term you want to use to
// act as: this is the right output, if anything changes its wrong. Within a 5%
// margin of error. I've no idea what is a good % to compare against here or
// even valid. I just want to know that the stuff I build renders roughly the
// same across platforms or not.
//
// Dumps all the reference images from `src/shaders/name.wesl` to
// `tests/fixtures/name_variant.png` to act as the reference image for each
// shader.
//
// Here to help with me building uimaterials vs regular materials in bevy. They
// only differ by input bindings so this whole thing exists cause mitch is a
// lazy bastard and got sick of copy/pasting crap all over.
use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::PathBuf;

use crate::render::RenderedFrame;

/// ssim (structural similarity index metric) threshold below which the test is
/// considered failed.
///
/// 0.95 is a pulled out of my butt 5% off as being ok. I have no idea what
/// makes sense, I'm not a gooey guy. But I figure if something is more than 5%
/// different its probably "wrong", whatever that means.
pub const DEFAULT_SSIM_THRESHOLD: f64 = 0.95;

/// Path to the committed fixture directory (inside the source tree so
/// snapshots survive `cargo clean`).
fn fixtures_dir() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set run under `cargo test`");
    PathBuf::from(manifest).join("tests").join("fixtures")
}

/// Convert a `RenderedFrame` to an `RgbaImage`, panicking on size mismatch.
pub fn frame_to_image(frame: &RenderedFrame) -> RgbaImage {
    ImageBuffer::from_raw(frame.width, frame.height, frame.pixels.clone())
        .expect("pixel buffer size does not match declared dimensions")
}

/// Build a per-pixel difference image from two same-size RGBA images.
///
/// Each output pixel encodes how wrong the rendered result is:
///   - Identical pixels are black aka no difference.
///   - Differing pixels are red, with intensity proportional to the maximum
///     absolute channel difference across R, G, and B, amplified 8x so even
///     small 1–2 unit diffs are clearly visible without needing to squint.
///   - Alpha is always 255 so the diff is fully opaque.
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
            let magnitude = (dr.max(dg).max(db).saturating_mul(8).min(255)) as u8;
            diff.put_pixel(x, y, Rgba([magnitude, 0, 0, 255]));
        }
    }
    diff
}

/// Stitch three same-size images side by side into one composite image:
///   expected | found | diff with red delta for the different pixels
///
/// A 2-pixel white separator is inserted between each panel so the boundaries
/// are unambiguous when all three panels have similar colors near their edges.
fn composite_diff(reference: &RgbaImage, rendered: &RgbaImage, diff: &RgbaImage) -> RgbaImage {
    let (w, h) = (reference.width(), reference.height());
    const SEP: u32 = 2;
    let total_w = w * 3 + SEP * 2;
    let mut out = RgbaImage::new(total_w, h);

    // Prefill the entire image with white before overlaying the images.
    for pixel in out.pixels_mut() {
        *pixel = Rgba([255, 255, 255, 255]);
    }

    let panels: [(&RgbaImage, u32); 3] =
        [(reference, 0), (rendered, w + SEP), (diff, w * 2 + SEP * 2)];
    for (src, x_off) in panels {
        for y in 0..h {
            for x in 0..w {
                out.put_pixel(x + x_off, y, *src.get_pixel(x, y));
            }
        }
    }
    out
}

/// Used to compare a rendered frame against a png reference snapshot in
/// `tests/fixtures/NAME.png`.
///
/// On first run with no reference file the rendered image is saved as the
/// reference. Pass `UPDATE_SNAPSHOTS=1` to force-overwrite an existing
/// reference.
///
/// When the SSIM score falls below `threshold` the test fails and a
/// composite PNG is written to `tests/fixtures/NAME_diff.png` containing
/// three panels side by side:
///
///   expected | found | red-diff
///
/// The diff panel amplifies differences 8x so even 1-unit per-channel errors
/// are clearly visible.
pub fn assert_snapshot(name: &str, frame: &RenderedFrame, threshold: f64) {
    let fixtures = fixtures_dir();
    let path = fixtures.join(format!("{name}.png"));

    let rendered = frame_to_image(frame);

    let update = std::env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !path.exists() || update {
        std::fs::create_dir_all(&fixtures).expect("could not create tests/fixtures/");
        rendered
            .save(&path)
            .unwrap_or_else(|e| panic!("could not save snapshot {path:?}: {e}"));
        if update {
            println!("updated: {path:?}");
        } else {
            println!("created initial reference: {path:?}");
        }
        return;
    }

    let reference = image::open(&path)
        .unwrap_or_else(|e| panic!("could not load reference snapshot {path:?}: {e}"))
        .to_rgba8();

    assert_eq!(
        (rendered.width(), rendered.height()),
        (reference.width(), reference.height()),
        "snapshot {name}: size mismatch - rendered {}x{} vs reference {}x{}",
        rendered.width(),
        rendered.height(),
        reference.width(),
        reference.height(),
    );

    let result = image_compare::rgba_hybrid_compare(&rendered, &reference)
        .unwrap_or_else(|e| panic!("ssim compare failed for {name}: {e}"));

    if result.score < threshold {
        let diff = diff_image(&rendered, &reference);
        let composite = composite_diff(&reference, &rendered, &diff);
        let diff_path = fixtures.join(format!("{name}_diff.png"));
        composite
            .save(&diff_path)
            .unwrap_or_else(|e| panic!("could not save diff image {diff_path:?}: {e}"));

        panic!(
            "snapshot {name}: ssim {:.4} < threshold {:.4}\n\
             diff saved to: {diff_path:?}\n\
             panels: expected | found | red-diff amplified 8x\n\
             rerun with UPDATE_SNAPSHOTS=1 to accept the new output, \
             or delete the reference PNG to recreate it from scratch",
            result.score, threshold,
        );
    }
}
