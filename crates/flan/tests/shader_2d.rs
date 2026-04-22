mod common;
use common::*;
use flan::snapshot::{DEFAULT_SSIM_THRESHOLD, assert_snapshot, frame_to_image};
use flan::wesl::Variant;

const PLOT_WESL: &str = include_str!("../src/2d/plot.wesl");
const REFERENCE_WESL: &str = include_str!("../src/2d/reference.wesl");

fn render_plot(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_wesl("2d/plot", PLOT_WESL, variant)
}

fn render_reference(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_wesl("2d/reference", REFERENCE_WESL, variant)
}

/// Reference render for plot material variant desktop, non-UI version.
#[test]
fn snapshot_plot_material() {
    let Some(frame) = render_plot(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("plot_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for plot ui variant desktop, UiMaterial binding logic.
#[test]
fn snapshot_plot_ui() {
    let Some(frame) = render_plot(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("plot_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for the reference shader material variant.
#[test]
fn snapshot_reference_material() {
    let Some(frame) = render_reference(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("reference_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for the reference shader ui variant.
#[test]
fn snapshot_reference_ui() {
    let Some(frame) = render_reference(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("reference_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Material and UI variants of plot should be visually identical - same shader
/// logic, only the binding group differs which WGPU_TEST normalises to @group(0).
#[test]
fn plot_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_plot(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_plot(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "plot: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

/// Material and UI variants of reference should be visually identical.
#[test]
fn reference_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_reference(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_reference(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "reference: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}
