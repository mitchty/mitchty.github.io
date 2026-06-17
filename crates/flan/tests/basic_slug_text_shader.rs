#![cfg(not(target_arch = "wasm32"))]

use flan::test::lib::shader::slug_text;
use flan::test::lib::snapshot::{
    DEFAULT_SSIM_THRESHOLD, assert_snapshot, assert_variant_matches_canonical,
};

const FIRA_TTF: &[u8] = include_bytes!("fixtures/FiraMono-Medium.ttf");

// Short string kept for snapshot comparisons.
const SHORT: &str = "Hi";
// Longer string used for the 3d visibility check.
const HELLO: &str = "Hello world!";

/// Headless Bevy render of [`SlugTextMaterial`] (storage-buffer path, @group(1)).
#[test]
#[cfg(not(feature = "webgl"))]
fn snapshot_slug_text_canonical() {
    let Some(frame) = slug_text::render_canonical(FIRA_TTF, SHORT) else {
        return;
    };
    assert_snapshot(slug_text::CANONICAL, &frame, DEFAULT_SSIM_THRESHOLD, None);
}

/// Headless Bevy render of [`SlugTextTextureMaterial`] (texture path, @group(1)).
/// Output must match the canonical within SSIM threshold.
#[test]
fn snapshot_slug_text_texture() {
    let Some(frame) = slug_text::render_texture(FIRA_TTF, SHORT) else {
        return;
    };
    assert_variant_matches_canonical(
        slug_text::CANONICAL,
        "slug_text_texture",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// Headless Bevy render of [`SlugText3dMaterial`] (storage-buffer path, @group(3)).
/// Uses a visibility check - fwidth-based pixels_per_em differs from the 2-D path.
#[test]
#[cfg(not(feature = "webgl"))]
fn slug_text3d_default_has_visible_text() {
    let Some(frame) = slug_text::render_3d_canonical(FIRA_TTF, HELLO) else {
        return;
    };
    assert!(
        has_dark_pixel(&frame.pixels, frame.width, frame.height),
        "slug_text3d_default: no dark pixels found 3d slug text3d default shader did not render"
    );
}

/// Headless Bevy render of [`SlugText3dTextureMaterial`] (texture path, @group(3)).
/// Uses a visibility check for the same reason as the storage-buffer 3d test.
#[test]
fn slug_text3d_texture_has_visible_text() {
    let Some(frame) = slug_text::render_3d_texture(FIRA_TTF, HELLO) else {
        return;
    };
    assert!(
        has_dark_pixel(&frame.pixels, frame.width, frame.height),
        "slug_text3d_texture: no dark pixels found - 3d slug text3d texture shader did not render"
    );
}

fn has_dark_pixel(pixels: &[u8], width: u32, height: u32) -> bool {
    for y in 0..height {
        for x in 0..width {
            let off = ((y * width + x) * 4) as usize;
            let [r, g, b, _a] = [
                pixels[off],
                pixels[off + 1],
                pixels[off + 2],
                pixels[off + 3],
            ];
            if (r as u32 + g as u32 + b as u32) < 600 {
                return true;
            }
        }
    }
    false
}
