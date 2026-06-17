// Integration tests for the slug text shader.
//
// The canonical render is the Bevy UiMaterial (storage-buffer) headless render,
// saved as the single blessed fixture at tests/fixtures/slug/canonical.png.
// Every other Bevy variant is compared against that fixture via
// assert_variant_matches_canonical.
#![cfg(not(target_arch = "wasm32"))]

use flan::test::lib::shader::slug;
use flan::test::lib::snapshot::{
    DEFAULT_SSIM_THRESHOLD, assert_snapshot, assert_variant_matches_canonical,
};

const FIRA_TTF: &[u8] = include_bytes!("fixtures/FiraMono-Medium.ttf");
const HELLO: &str = "Hello world!";
const SHORT: &str = "Hi";

/// Headless Bevy render of `SlugMaterial` as `UiMaterial` (@group(1), SSB).
#[test]
#[cfg(not(feature = "webgl"))]
fn snapshot_slug_canonical() {
    let Some(frame) = slug::render_canonical(FIRA_TTF, SHORT) else {
        return;
    };
    assert_snapshot(slug::CANONICAL, &frame, DEFAULT_SSIM_THRESHOLD, None);
}

/// Bevy `MeshMaterial3d<SlugMaterial>` (@group(3)) headless render.
#[test]
#[cfg(not(feature = "webgl"))]
fn snapshot_slug_bevy_material() {
    let Some(frame) = slug::render_bevy_material(FIRA_TTF, SHORT) else {
        return;
    };
    assert_variant_matches_canonical(
        slug::CANONICAL,
        "slug_bevy_material",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// Bevy `UiMaterial` (@group(1)) headless render.
#[test]
#[cfg(not(feature = "webgl"))]
fn snapshot_slug_bevy_ui_material() {
    let Some(frame) = slug::render_bevy_ui_material(FIRA_TTF, SHORT) else {
        return;
    };
    assert_variant_matches_canonical(
        slug::CANONICAL,
        "slug_bevy_ui_material",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// Bevy `UiMaterial` via `SlugMaterialTexture` + `slug_ui_material_webgl`.
/// Full Bevy pipeline, texture bindings - mirrors FPS overlay webgl path.
#[test]
fn snapshot_slug_bevy_ui_material_texture() {
    let Some(frame) = slug::render_bevy_ui_material_texture(FIRA_TTF, SHORT) else {
        return;
    };
    assert_variant_matches_canonical(
        slug::CANONICAL,
        "slug_bevy_ui_material_texture",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// Bevy `MeshMaterial3d` via `SlugMaterialTexture` + `slug_material_3d_webgl`.
/// Full Bevy 3d material pipeline, texture bindings - mirrors world-space slug
/// text webgl path.
///
/// Uses a visibility check rather than SSIM: the 3d path uses `fwidth`-based
/// pixels_per_em which produces slightly different sub-pixel coverage than the
/// 2-D `slugtext()` path, so pixel-level comparison against the 2-D canonical
/// would be unreliable. We just assert that the shader actually rendered glyph
/// pixels (non-background output is present).
#[test]
fn slug_bevy_material_texture_has_visible_text() {
    let Some(frame) = slug::render_bevy_material_texture(FIRA_TTF, HELLO) else {
        return;
    };
    assert!(
        has_dark_pixel(&frame.pixels, frame.width, frame.height),
        "slug_bevy_material_texture: no dark pixels found - 3d slug material did not render"
    );
}

/// Render "00.0 fps" at font_size=18.0 in a 130x24 node with Right/Center
/// layout through the full `SlugPlugin` system chain, including `sync_node_size`.
///
/// This exactly mirrors how mitchty's FPS display spawns its slug text entity.
/// It exercises `sync_node_size` (which sets `node_size` from `ComputedNode`)
/// rather than hardcoding it in params - the critical difference from the other
/// Bevy tests. If `sync_node_size` fails to fire, `node_size` stays (0, 0)
/// and the shader renders nothing.
#[test]
fn slug_fps_like_has_visible_text() {
    let Some(frame) = slug::render_fps_like(FIRA_TTF) else {
        return;
    };
    assert!(
        has_dark_pixel(&frame.pixels, frame.width, frame.height),
        "slug_fps_like: no dark pixel text in node. \
         Check sync_node_size fires and node_size is non-zero."
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
