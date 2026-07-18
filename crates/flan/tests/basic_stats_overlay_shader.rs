#![cfg(not(target_arch = "wasm32"))]

use flan::test::lib::shader::stats_overlay;
use flan::test::lib::snapshot::{
    DEFAULT_SSIM_THRESHOLD, assert_snapshot, assert_variant_matches_canonical,
    has_non_background_pixels,
};

const FONT: &[u8] = include_bytes!("fixtures/FiraMono-Medium.ttf");
const FPS_TEXT: &str = "120.0 fps";

/// 256 pre-averaged FPS values a sine wave between 30 and 90 fps.
fn test_fps_values() -> [f32; 256] {
    let mut v = [60.0f32; 256];
    for (i, x) in v.iter_mut().enumerate() {
        *x = 60.0 + 30.0 * (i as f32 / 255.0 * std::f32::consts::TAU).sin();
    }
    v
}

#[test]
fn snapshot_stats_overlay_canonical() {
    let fps = test_fps_values();
    let Some(frame) = stats_overlay::render_canonical(FONT, &fps, FPS_TEXT) else {
        return;
    };
    assert_snapshot(
        stats_overlay::CANONICAL,
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

#[test]
fn snapshot_stats_overlay_texture() {
    let fps = test_fps_values();
    let Some(frame) = stats_overlay::render_texture(FONT, &fps, FPS_TEXT) else {
        return;
    };
    assert_variant_matches_canonical(
        stats_overlay::CANONICAL,
        "stats_overlay_texture",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// `StatsOverlayColorMode::Invert` GPU inversion path needed proof it works.
#[test]
fn snapshot_stats_overlay_invert_canonical() {
    let fps = test_fps_values();
    let Some(frame) = stats_overlay::render_invert_canonical(FONT, &fps, FPS_TEXT) else {
        return;
    };
    assert!(
        has_non_background_pixels(&frame),
        "invert mode path rendered a fully transparent frame"
    );
    assert_snapshot(
        stats_overlay::INVERT_CANONICAL,
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}

/// Texture variant of above.
#[test]
fn snapshot_stats_overlay_invert_texture() {
    let fps = test_fps_values();
    let Some(frame) = stats_overlay::render_invert_texture(FONT, &fps, FPS_TEXT) else {
        return;
    };
    assert!(
        has_non_background_pixels(&frame),
        "invert mode via texture path rendered a fully transparent frame"
    );
    assert_variant_matches_canonical(
        stats_overlay::INVERT_CANONICAL,
        "stats_overlay_invert_texture",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}
