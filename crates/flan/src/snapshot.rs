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
use image::{ImageBuffer, RgbaImage};
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

/// Used to compare a rendered frame against a png reference snapshot in
/// `tests/fixtures/NAME.png`
///
/// Abuses ssim threshold to determine if things are too far apart or not.
pub fn assert_snapshot(name: &str, frame: &RenderedFrame, threshold: f64) {
    let fixtures = fixtures_dir();
    let path = fixtures.join(format!("{name}.png"));

    let rendered = frame_to_image(frame);

    // I"m not overly keen on this approach. Might remove it later.
    let update = std::env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !path.exists() || update {
        std::fs::create_dir_all(&fixtures).expect("could not create tests/fixtures/");
        rendered
            .save(&path)
            .unwrap_or_else(|e| panic!("could not save snapshot {path:?}: {e}"));

        // TODO: Nuke?
        if update {
            println!("updated: {path:?}");
        } else {
            println!("created initial reference: {path:?}");
        }
        return;
    }

    // Load the reference image and compare to whatever got rendered.
    let reference = image::open(&path)
        .unwrap_or_else(|e| panic!("could not load reference snapshot {path:?}: {e}"))
        .to_rgba8();

    assert_eq!(
        (rendered.width(), rendered.height()),
        (reference.width(), reference.height()),
        "snapshot reference {name}: rendered {}x{} != reference {}x{}",
        rendered.width(),
        rendered.height(),
        reference.width(),
        reference.height(),
    );

    // TODO: I wonder if there is a way to abuse those terminals that output
    // images directly to show a visual diff with this... FUTURE MITCH PROBLEM!
    let result = image_compare::rgba_hybrid_compare(&rendered, &reference)
        .unwrap_or_else(|e| panic!("ssim failed for {name}: {e}"));

    assert!(
        result.score >= threshold,
        "snapshot {name}: ssim {:.4} < threshold {:.4} rerun with UPDATE_SNAPSHOTS=anything to regenerate if this is intentional, or just remove the dam reference image",
        result.score,
        threshold,
    );
}
