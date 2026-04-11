mod common;
use common::*;
use flan::snapshot::{DEFAULT_SSIM_THRESHOLD, assert_snapshot, frame_to_image};
use flan::wesl::Variant;

const CHROMATIC_ABERRATION_WESL: &str = include_str!("../src/fullscreen/chromatic-aberration.wesl");
const VHS_EFFECT_WESL: &str = include_str!("../src/fullscreen/vhs-effect.wesl");
const EM_INTERFERENCE_WESL: &str = include_str!("../src/fullscreen/em-interference.wesl");
const OIL_PAINTING_WESL: &str = include_str!("../src/fullscreen/oil-painting.wesl");
const EDGE_CARTOON_WESL: &str = include_str!("../src/fullscreen/edge-cartoon.wesl");

fn render_chromatic_aberration(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_fullscreen_effect(
        "fullscreen/chromatic-aberration",
        CHROMATIC_ABERRATION_WESL,
        variant,
    )
}

fn render_vhs_effect(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_fullscreen_effect("fullscreen/vhs-effect", VHS_EFFECT_WESL, variant)
}

fn render_em_interference(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_fullscreen_effect("fullscreen/em-interference", EM_INTERFERENCE_WESL, variant)
}

fn render_oil_painting(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_fullscreen_effect("fullscreen/oil-painting", OIL_PAINTING_WESL, variant)
}

fn render_edge_cartoon(variant: Variant) -> Option<flan::render::RenderedFrame> {
    render_fullscreen_effect("fullscreen/edge-cartoon", EDGE_CARTOON_WESL, variant)
}

#[test]
fn snapshot_chromatic_aberration_material() {
    let Some(frame) = render_chromatic_aberration(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot(
        "chromatic_aberration_material",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
    );
}

#[test]
fn snapshot_chromatic_aberration_ui() {
    let Some(frame) = render_chromatic_aberration(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("chromatic_aberration_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn chromatic_aberration_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_chromatic_aberration(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_chromatic_aberration(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "chromatic-aberration: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

#[test]
fn snapshot_vhs_effect_material() {
    let Some(frame) = render_vhs_effect(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("vhs_effect_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn snapshot_vhs_effect_ui() {
    let Some(frame) = render_vhs_effect(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("vhs_effect_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn vhs_effect_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_vhs_effect(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_vhs_effect(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "vhs-effect: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

#[test]
fn snapshot_em_interference_material() {
    let Some(frame) = render_em_interference(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("em_interference_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn snapshot_em_interference_ui() {
    let Some(frame) = render_em_interference(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("em_interference_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn em_interference_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_em_interference(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_em_interference(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "em-interference: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

#[test]
fn snapshot_oil_painting_material() {
    let Some(frame) = render_oil_painting(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("oil_painting_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn snapshot_oil_painting_ui() {
    let Some(frame) = render_oil_painting(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("oil_painting_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn oil_painting_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_oil_painting(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_oil_painting(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "oil-painting: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

#[test]
fn snapshot_edge_cartoon_material() {
    let Some(frame) = render_edge_cartoon(Variant::TEST_MATERIAL) else {
        return;
    };
    assert_snapshot("edge_cartoon_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn snapshot_edge_cartoon_ui() {
    let Some(frame) = render_edge_cartoon(Variant::TEST_UI) else {
        return;
    };
    assert_snapshot("edge_cartoon_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

#[test]
fn edge_cartoon_material_and_ui_are_visually_equivalent() {
    let Some(mat_frame) = render_edge_cartoon(Variant::TEST_MATERIAL) else {
        return;
    };
    let Some(ui_frame) = render_edge_cartoon(Variant::TEST_UI) else {
        return;
    };

    let result =
        image_compare::rgba_hybrid_compare(&frame_to_image(&mat_frame), &frame_to_image(&ui_frame))
            .expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "edge-cartoon: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}
