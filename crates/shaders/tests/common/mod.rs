// Shared test helpers for shader_* integration tests.
//
// Not every test binary in the test framework uses everything. Silence unused
// warnings.
#![allow(dead_code)]
use bytemuck::{Pod, Zeroable};
use shaders::render::{Binding, BindingKind, render_shader};
use shaders::wesl::{Variant, compile};

// Non-WebGL uniform layout 40 bytes, no padding.
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
pub struct PlotUniform {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub zoom: [f32; 2],
    pub offset: [f32; 2],
    pub count: u32,
    pub time: f32,
}

impl PlotUniform {
    pub fn test_default() -> Self {
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

// WebGL uniform layout 48 bytes
// multiple of 16 bytes; count u32 + time f32 = 8 bytes at offset 32, so we
// need 8 bytes of padding to get to 48.
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
pub struct PlotUniformWebGl {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub zoom: [f32; 2],
    pub offset: [f32; 2],
    pub count: u32,
    pub time: f32,
    pub _pad: [u32; 2],
}

impl PlotUniformWebGl {
    pub fn test_default() -> Self {
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

// WGPU_TEST variant: a single uniform buffer matching FullscreenEffectSettings.
//
// Non-WebGL 8 bytes:
//   struct FullscreenEffectSettings {
//       intensity: f32,   // offset 0 (4 bytes)
//       time:      f32,   // offset 4 (4 bytes)
//   }                     // total: 8 bytes
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FullscreenEffectUniform {
    pub intensity: f32,
    pub time: f32,
}

impl FullscreenEffectUniform {
    pub fn test_default() -> Self {
        // intensity=5.0 -> clamp(5.0/10.0)=0.5 -> medium grey in WGPU_TEST mode
        Self {
            intensity: 5.0,
            time: 0.0,
        }
    }
}

// WebGL 16 bytes:
//   struct FullscreenEffectSettingsWebGl {
//       intensity:       f32,         // offset  0 (4 bytes)
//       time:            f32,         // offset  4 (4 bytes)
//       _webgl2_padding: vec2<f32>,   // offset  8 (8 bytes padding)
//   }                                 // total: 16 bytes
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FullscreenEffectUniformWebGl {
    pub intensity: f32,
    pub time: f32,
    pub _pad: [f32; 2],
}

impl FullscreenEffectUniformWebGl {
    pub fn test_default() -> Self {
        Self {
            intensity: 5.0,
            time: 0.0,
            _pad: [0.0; 2],
        }
    }
}

// 200 points keeps WebGL count well within MAX_PLOT_POINTS 512 and gives
// enough density that the polyline SDF matches the analytic curve closely
// enough to pass the SSIM threshold for now in the unit tests.
pub const TEST_POINT_COUNT: usize = 200;

pub fn test_sin_wave_points() -> Vec<[f32; 2]> {
    (0..TEST_POINT_COUNT)
        .map(|i| {
            let x = i as f32 / (TEST_POINT_COUNT - 1) as f32;
            let y = (x * 10.0_f32).sin() * 0.4 + 0.5;
            [x, y]
        })
        .collect()
}

pub fn test_points_bytes() -> Vec<u8> {
    bytemuck::cast_slice(&test_sin_wave_points()).to_vec()
}

/// Returns true when the error string indicates no GPU adapter was found.
/// Used by all render helpers to skip gracefully in headless CI.
pub fn has_no_adapter(err: &str) -> bool {
    err.contains("no wgpu adapter found")
}

/// Compile and render a 2d shader (two bindings: uniform params + points buffer).
///
/// `stem` e.g. `"2d/plot"`.  `variant` must be a `TEST_*` variant.
///
/// Returns `None` when no GPU adapter is found; panics on any other error.
pub fn render_wesl(
    stem: &str,
    src: &str,
    variant: Variant,
) -> Option<shaders::render::RenderedFrame> {
    assert!(variant.wgpu_test, "render_wesl requires a TEST_* variant");

    let wgsl = compile(stem, src, variant)
        .unwrap_or_else(|e| panic!("WESL compile failed for {stem}: {e}"));

    let result = if variant.webgl {
        let uniform = PlotUniformWebGl::test_default();
        // WEBGL: binding 1 is a uniform buffer of 512 x vec4<f32>.
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
                    data: bytemuck::bytes_of(&uniform),
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
        let point_bytes = test_points_bytes();
        render_shader(
            &wgsl,
            &[
                Binding {
                    slot: 0,
                    kind: BindingKind::Uniform,
                    data: bytemuck::bytes_of(&uniform),
                },
                Binding {
                    slot: 1,
                    kind: BindingKind::StorageRead,
                    data: &point_bytes,
                },
            ],
        )
    };

    match result {
        Ok(frame) => Some(frame),
        Err(ref e) if has_no_adapter(e) => {
            eprintln!("skipping {stem}/{} no gpu found: {e}", variant.dir_name());
            None
        }
        Err(e) => panic!("render({stem}, {}) failed: {e}", variant.dir_name()),
    }
}

/// Compile and render a fullscreen post-process shader (single uniform binding).
///
/// `stem` e.g. `"fullscreen/vhs-effect"`.  `variant` must be a `TEST_*` variant.
///
/// Returns `None` when no GPU adapter is found; panics on any other error.
pub fn render_fullscreen_effect(
    stem: &str,
    src: &str,
    variant: Variant,
) -> Option<shaders::render::RenderedFrame> {
    assert!(
        variant.wgpu_test,
        "render_fullscreen_effect requires a TEST_* variant"
    );

    let wgsl = compile(stem, src, variant)
        .unwrap_or_else(|e| panic!("WESL compile failed for {stem}: {e}"));

    let result = if variant.webgl {
        let uniform = FullscreenEffectUniformWebGl::test_default();
        render_shader(
            &wgsl,
            &[Binding {
                slot: 0,
                kind: BindingKind::Uniform,
                data: bytemuck::bytes_of(&uniform),
            }],
        )
    } else {
        let uniform = FullscreenEffectUniform::test_default();
        render_shader(
            &wgsl,
            &[Binding {
                slot: 0,
                kind: BindingKind::Uniform,
                data: bytemuck::bytes_of(&uniform),
            }],
        )
    };

    match result {
        Ok(frame) => Some(frame),
        Err(ref e) if has_no_adapter(e) => {
            eprintln!("skipping {stem}/{} no gpu found: {e}", variant.dir_name());
            None
        }
        Err(e) => panic!("render({stem}, {}) failed: {e}", variant.dir_name()),
    }
}
