// Integration tests for the plot shader.
//
// The canonical render is the PlotUiMaterial storage-buffer headless render,
// saved as the blessed fixture at tests/fixtures/plot/canonical.png.
// The texture variant is compared against that fixture via SSIM.
#![cfg(not(target_arch = "wasm32"))]

use flan::test::lib::shader::plot;
use flan::test::lib::snapshot::{
    DEFAULT_SSIM_THRESHOLD, assert_snapshot, assert_variant_matches_canonical,
};

#[test]
fn snapshot_plot_canonical() {
    let Some(frame) = plot::render_canonical() else {
        return;
    };
    assert_snapshot(plot::CANONICAL, &frame, DEFAULT_SSIM_THRESHOLD, None);
}

#[test]
fn snapshot_plot_texture() {
    let Some(frame) = plot::render_texture() else {
        return;
    };
    assert_variant_matches_canonical(
        plot::CANONICAL,
        "plot_texture",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
        None,
    );
}
