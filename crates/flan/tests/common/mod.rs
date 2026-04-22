// Shared test helpers for shader_* integration tests.
//
// Not every test binary in the test framework uses everything. Silence unused
// warnings.
#![allow(dead_code)]
use bevy::math::Vec2;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::encase::UniformBuffer;
use flan::render::{Binding, BindingKind, render_shader};
use flan::wesl::{Variant, compile};

/// Write a ShaderType value into a byte vec and zero-pad to the struct's
/// min_size() so the buffer matches what the WGSL struct declares.
///
/// Here because `encase::UniformBuffer::write()` uses the runtime size() of the
/// struct, which for a struct whose last field has #[shader(size(N))] only
/// writes the field's natural bytes, not padded N bytes. min_size() correctly
/// reflects the padded layout, so we extend to that length with this so wgpu is
/// happy on platforms that need 16 byte alignment. e.g. webgl and to a degree
/// metal shaders as far as what gets passed into the gpu. Note metal only needs
/// the gpu side to align not the struct itself. Shaders are a bit of a
/// shitshow.
fn write_padded<T: ShaderType + bevy::render::render_resource::encase::internal::WriteInto>(
    val: &T,
) -> Vec<u8> {
    let mut buf = UniformBuffer::new(Vec::new());
    buf.write(val).expect("encase write failed");
    let mut bytes = buf.into_inner();
    let min = T::min_size().get() as usize;
    if bytes.len() < min {
        bytes.resize(min, 0);
    }
    bytes
}

/// Uniform buffer struct mirroring the WESL shader's PlotUniform layout.
#[derive(ShaderType)]
pub struct PlotUniform {
    pub min: Vec2,
    pub max: Vec2,
    pub zoom: Vec2,
    pub offset: Vec2,
    pub count: u32,
    pub time: f32,
    // Mirror the #[shader(size(8))] on the Bevy-side PlotUniform so the bytes
    // written by encase match the WGSL @size(8) declaration struct = 48 bytes.
    #[shader(size(8))]
    pub line_width: f32,
}

impl PlotUniform {
    pub fn test_default() -> Self {
        Self {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            zoom: Vec2::ONE,
            offset: Vec2::ZERO,
            count: TEST_POINT_COUNT as u32,
            time: 0.0,
            line_width: 0.003,
        }
    }

    pub fn to_wgsl_bytes(&self) -> Vec<u8> {
        write_padded(self)
    }
}

/// Uniform buffer struct mirroring FullscreenEffectSettings.
///
/// Same deal - encase handles alignment for both WebGL and native paths.
#[derive(ShaderType)]
pub struct FullscreenEffectUniform {
    pub intensity: f32,
    // Mirror the #[shader(size(12))] on PostProcessSettings so that min_size()
    // reports 16 bytes making wgpu's WebGL binding validation to match.
    #[shader(size(12))]
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

    pub fn to_wgsl_bytes(&self) -> Vec<u8> {
        write_padded(self)
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

/// Raw bytes for the non-WebGL storage buffer (plain tightly-packed vec2<f32>).
pub fn test_points_bytes() -> Vec<u8> {
    test_sin_wave_points()
        .iter()
        .flat_map(|p| p[0].to_le_bytes().into_iter().chain(p[1].to_le_bytes()))
        .collect()
}

/// Returns true when the error string indicates no GPU adapter was found.
/// Used by all render helpers to skip gracefully in headless CI.
pub fn has_no_adapter(err: &str) -> bool {
    err.contains("no wgpu adapter found")
}

/// Compile and render a 2d shader (two bindings: uniform params + points buffer).
///
/// `stem` e.g. `"2d/plot"`. `variant` must be a `TEST_*` variant.
///
/// Returns `None` when no GPU adapter is found; panics on any other error.
pub fn render_wesl(stem: &str, src: &str, variant: Variant) -> Option<flan::render::RenderedFrame> {
    assert!(variant.wgpu_test, "render_wesl requires a TEST_* variant");

    let wgsl = compile(stem, src, variant)
        .unwrap_or_else(|e| panic!("WESL compile failed for {stem}: {e}"));

    eprintln!(
        "=== Compiled WGSL for {} (webgl={}, ui_material={}, wgpu_test={}) ===",
        stem, variant.webgl, variant.ui_material, variant.wgpu_test
    );
    eprintln!("{}", wgsl);
    eprintln!("=== END WGSL ===");

    // encase serialises PlotUniform with correct std140 padding for both paths.
    let uniform_bytes = PlotUniform::test_default().to_wgsl_bytes();

    let result = if variant.webgl {
        // WEBGL: binding 1 is a uniform buffer of 512 x vec4<f32>.
        // Each point occupies one vec4 (16 bytes): xy = data, zw = 0.
        let mut point_data = vec![0u8; 512 * 4 * 4];
        for (i, p) in test_sin_wave_points().iter().enumerate() {
            let base = i * 16;
            point_data[base..base + 4].copy_from_slice(&p[0].to_le_bytes());
            point_data[base + 4..base + 8].copy_from_slice(&p[1].to_le_bytes());
            // zw remain zero
        }
        render_shader(
            &wgsl,
            &[
                Binding {
                    slot: 0,
                    kind: BindingKind::Uniform,
                    data: &uniform_bytes,
                },
                Binding {
                    slot: 1,
                    kind: BindingKind::Uniform,
                    data: &point_data,
                },
            ],
        )
    } else {
        // Non-WEBGL: binding 1 is a storage buffer of vec2<f32>.
        let point_bytes = test_points_bytes();
        render_shader(
            &wgsl,
            &[
                Binding {
                    slot: 0,
                    kind: BindingKind::Uniform,
                    data: &uniform_bytes,
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
/// `stem` e.g. `"fullscreen/vhs-effect"`. `variant` must be a `TEST_*` variant.
///
/// Returns `None` when no GPU adapter is found; panics on any other error.
pub fn render_fullscreen_effect(
    stem: &str,
    src: &str,
    variant: Variant,
) -> Option<flan::render::RenderedFrame> {
    assert!(
        variant.wgpu_test,
        "render_fullscreen_effect requires a TEST_* variant"
    );

    let wgsl = compile(stem, src, variant)
        .unwrap_or_else(|e| panic!("WESL compile failed for {stem}: {e}"));

    // Same struct for both WebGL and non-WebGL; encase pads correctly either
    // way. write_padded ensures the buffer is min_size() bytes ~16 so that
    // wgpu's WebGL binding validation passes.
    let bytes = FullscreenEffectUniform::test_default().to_wgsl_bytes();

    let result = render_shader(
        &wgsl,
        &[Binding {
            slot: 0,
            kind: BindingKind::Uniform,
            data: &bytes,
        }],
    );

    match result {
        Ok(frame) => Some(frame),
        Err(ref e) if has_no_adapter(e) => {
            eprintln!("skipping {stem}/{} no gpu found: {e}", variant.dir_name());
            None
        }
        Err(e) => panic!("render({stem}, {}) failed: {e}", variant.dir_name()),
    }
}
