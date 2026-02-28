// TODO: Refactor this to be dynamic mabye?
use bytemuck::{Pod, Zeroable};
use shaders::render::{Binding, BindingKind, render_shader};
use shaders::snapshot::{DEFAULT_SSIM_THRESHOLD, assert_snapshot, frame_to_image};
use shaders::wesl::{Variant, compile};

const PLOT_WESL: &str = include_str!("../src/shaders/plot.wesl");
const REFERENCE_WESL: &str = include_str!("../src/shaders/reference.wesl");

// Non-WebGL uniform layout — 40 bytes, no padding.
//   struct PlotUniform {
//       min:    vec2<f32>,   // offset  0  (8 bytes)
//       max:    vec2<f32>,   // offset  8  (8 bytes)
//       zoom:   vec2<f32>,   // offset 16  (8 bytes)
//       offset: vec2<f32>,   // offset 24  (8 bytes)
//       count:  u32,         // offset 32  (4 bytes)
//       time:   f32,         // offset 36  (4 bytes)
//   }                        // total: 40 bytes
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PlotUniform {
    min: [f32; 2],
    max: [f32; 2],
    zoom: [f32; 2],
    offset: [f32; 2],
    count: u32,
    time: f32,
}

impl PlotUniform {
    fn test_default() -> Self {
        Self {
            min: [0.0, 0.0],
            max: [1.0, 1.0],
            zoom: [1.0, 1.0],
            offset: [0.0, 0.0],
            count: TEST_POINT_COUNT as u32,
            time: 0.0,
        }
    }
}

// WebGL uniform layout — 48 bytes (std140 requires struct size to be a
// multiple of 16 bytes; count u32 + time f32 = 8 bytes at offset 32, so we
// only need 8 bytes of padding to reach 48).
//   struct PlotUniformWebGl {
//       min:    vec2<f32>,   // offset  0  (8 bytes)
//       max:    vec2<f32>,   // offset  8  (8 bytes)
//       zoom:   vec2<f32>,   // offset 16  (8 bytes)
//       offset: vec2<f32>,   // offset 24  (8 bytes)
//       count:  u32,         // offset 32  (4 bytes)
//       time:   f32,         // offset 36  (4 bytes)
//       _pad:   [u32; 2],    // offset 40  (8 bytes padding)
//   }                        // total: 48 bytes
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PlotUniformWebGl {
    min: [f32; 2],
    max: [f32; 2],
    zoom: [f32; 2],
    offset: [f32; 2],
    count: u32,
    time: f32,
    _pad: [u32; 2],
}

impl PlotUniformWebGl {
    fn test_default() -> Self {
        Self {
            min: [0.0, 0.0],
            max: [1.0, 1.0],
            zoom: [1.0, 1.0],
            offset: [0.0, 0.0],
            count: TEST_POINT_COUNT as u32,
            time: 0.0,
            _pad: [0; 2],
        }
    }
}

// Sample the same sin wave the old analytic shader drew so reference PNGs
// remain valid.  200 points keeps WebGL count well within MAX_PLOT_POINTS (512)
// and gives enough density that the polyline SDF matches the analytic one
// closely enough to pass the SSIM threshold.
const TEST_POINT_COUNT: usize = 200;

fn test_sin_wave_points() -> Vec<[f32; 2]> {
    (0..TEST_POINT_COUNT)
        .map(|i| {
            let x = i as f32 / (TEST_POINT_COUNT - 1) as f32;
            let y = (x * 10.0_f32).sin() * 0.4 + 0.5;
            [x, y]
        })
        .collect()
}

fn test_points_bytes() -> Vec<u8> {
    let pts = test_sin_wave_points();
    bytemuck::cast_slice(&pts).to_vec()
}

/// Compile `src` (a raw WESL string) for `variant` (must be a WGPU_TEST
/// variant) and render it with the standard test uniform data.
///
/// Uses `Variant::TEST_*` so the WESL produces @group(0) bindings and a
/// minimal FragmentInput — no group patching required.
fn render_wesl(stem: &str, src: &str, variant: Variant) -> shaders::render::RenderedFrame {
    assert!(variant.wgpu_test, "render_wesl requires a TEST_* variant");

    let wgsl = compile(stem, src, variant)
        .unwrap_or_else(|e| panic!("WESL compile failed for {stem}: {e}"));

    let frame = if variant.webgl {
        let uniform = PlotUniformWebGl::test_default();
        let uniform_bytes = bytemuck::bytes_of(&uniform);
        // WEBGL: binding 1 is a uniform buffer of 512 × vec4<f32>.
        // Pack sin-wave test points into the first TEST_POINT_COUNT vec4 slots
        // (.xy = point, .zw = 0).
        let mut point_data = vec![0u32; 512 * 4];
        for (i, p) in test_sin_wave_points().iter().enumerate() {
            point_data[i * 4] = p[0].to_bits();
            point_data[i * 4 + 1] = p[1].to_bits();
        }
        let point_bytes: &[u8] = bytemuck::cast_slice(&point_data);
        render_shader(
            &wgsl,
            &[
                Binding {
                    slot: 0,
                    kind: BindingKind::Uniform,
                    data: uniform_bytes,
                },
                Binding {
                    slot: 1,
                    kind: BindingKind::Uniform,
                    data: point_bytes,
                },
            ],
        )
    } else {
        // Non-WEBGL: binding 1 is a storage buffer of vec2<f32>.
        let uniform = PlotUniform::test_default();
        let uniform_bytes = bytemuck::bytes_of(&uniform);
        let point_bytes = test_points_bytes();
        render_shader(
            &wgsl,
            &[
                Binding {
                    slot: 0,
                    kind: BindingKind::Uniform,
                    data: uniform_bytes,
                },
                Binding {
                    slot: 1,
                    kind: BindingKind::StorageRead,
                    data: &point_bytes,
                },
            ],
        )
    };

    frame.unwrap_or_else(|e| panic!("render({stem}, {:?}) failed: {e}", variant.dir_name()))
}

fn render_plot(variant: Variant) -> shaders::render::RenderedFrame {
    render_wesl("plot", PLOT_WESL, variant)
}

fn render_reference(variant: Variant) -> shaders::render::RenderedFrame {
    render_wesl("reference", REFERENCE_WESL, variant)
}

/// Reference render for plot — material variant (desktop, non-UI).
#[test]
fn snapshot_plot_material() {
    let frame = render_plot(Variant::TEST_MATERIAL);
    assert_snapshot("plot_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for plot — ui variant (desktop, UiMaterial binding logic).
#[test]
fn snapshot_plot_ui() {
    let frame = render_plot(Variant::TEST_UI);
    assert_snapshot("plot_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for the reference shader — material variant.
#[test]
fn snapshot_reference_material() {
    let frame = render_reference(Variant::TEST_MATERIAL);
    assert_snapshot("reference_material", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Reference render for the reference shader — ui variant.
#[test]
fn snapshot_reference_ui() {
    let frame = render_reference(Variant::TEST_UI);
    assert_snapshot("reference_ui", &frame, DEFAULT_SSIM_THRESHOLD);
}

/// Material and UI variants of plot should be visually identical — same shader
/// logic, only the binding group differs (which WGPU_TEST normalises to
/// @group(0)).
#[test]
fn plot_material_and_ui_are_visually_equivalent() {
    let mat = frame_to_image(&render_plot(Variant::TEST_MATERIAL));
    let ui = frame_to_image(&render_plot(Variant::TEST_UI));

    let result = image_compare::rgba_hybrid_compare(&mat, &ui).expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "plot: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}

/// Material and UI variants of reference should be visually identical.
#[test]
fn reference_material_and_ui_are_visually_equivalent() {
    let mat = frame_to_image(&render_reference(Variant::TEST_MATERIAL));
    let ui = frame_to_image(&render_reference(Variant::TEST_UI));

    let result = image_compare::rgba_hybrid_compare(&mat, &ui).expect("SSIM comparison failed");

    assert!(
        result.score >= DEFAULT_SSIM_THRESHOLD,
        "reference: material and ui variants diverge visually: SSIM = {:.4}",
        result.score,
    );
}
