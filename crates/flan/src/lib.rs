use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

/// Maximum number of plot points stored in the WebGL2 uniform buffer.
/// Must match the array size in the WESL shader source (`array<vec4<f32>, 512>`).
pub const MAX_PLOT_POINTS: usize = 512;

pub struct PlotPlugin;

impl Plugin for PlotPlugin {
    fn build(&self, app: &mut App) {
        // ShadersPlugin must already have been added by the consuming app
        // (mitchty adds it in main before PlotPlugin).  We only need to
        // register the Material2d pipeline here.
        app.add_plugins(Material2dPlugin::<PlotMaterial>::default());
    }
}

/// Uniform data shared between the Rust side and every plot shader variant.
///
/// `count` is only consumed by the WEBGL path (the storage-buffer path uses
/// `arrayLength` instead), but it is always present so the WGSL struct layout
/// is identical across all variants.
#[derive(Clone, Copy, ShaderType)]
pub struct PlotUniform {
    pub min: Vec2,
    pub max: Vec2,
    pub zoom: Vec2,
    pub offset: Vec2,
    /// Number of valid points in the `points` array.  Set this to the actual
    /// point count on every update; ignored on non-WebGL builds at runtime.
    pub count: u32,
    // WebGL2 structs must be 16 byte aligned
    #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
    pub _webgl2_padding: Vec2,
}

/// Points buffer used in WebGL2 builds (uniform buffer, WEBGL feature).
///
/// Uses `Vec4` so that the Rust layout and the WGSL `array<vec4<f32>, 512>`
/// layout agree exactly under std140 (WebGL2 GLSL uniform block rules).
/// The `.xy` components hold the actual `(x, y)` data; `.zw` are unused.
///
/// Must match `MAX_PLOT_POINTS` and the WESL shader constant.
#[cfg(feature = "webgl")]
#[derive(Clone, ShaderType)]
pub struct PlotPointsUniform {
    pub data: [Vec4; MAX_PLOT_POINTS],
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct PlotMaterial {
    #[uniform(0)]
    pub params: PlotUniform,

    /// Storage buffer (native / WebGPU).
    #[cfg(not(feature = "webgl"))]
    #[storage(1, read_only)]
    pub points: Handle<ShaderStorageBuffer>,

    /// Uniform buffer (WebGL2).
    #[cfg(feature = "webgl")]
    #[uniform(1)]
    pub points: PlotPointsUniform,
}

impl Material2d for PlotMaterial {
    fn fragment_shader() -> ShaderRef {
        #[cfg(not(feature = "webgl"))]
        {
            shaders::BEVY_DEFAULT_MATERIAL_PLOT.clone().into()
        }
        #[cfg(feature = "webgl")]
        {
            shaders::BEVY_WEBGL_MATERIAL_PLOT.clone().into()
        }
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
    fn plot_uniform_webgl_alignment() {
        use std::mem::size_of;
        // WebGL requires uniform buffer structs to be multiples of 16 bytes
        assert_eq!(
            size_of::<PlotUniform>() % 16,
            0,
            "PlotUniform must be a multiple of 16 bytes for WebGL (got {} bytes)",
            size_of::<PlotUniform>()
        );
    }

    #[test]
    #[cfg(feature = "webgl")]
    fn plot_points_uniform_webgl_alignment() {
        use std::mem::size_of;
        // WebGL requires uniform buffer structs to be multiples of 16 bytes
        assert_eq!(
            size_of::<PlotPointsUniform>() % 16,
            0,
            "PlotPointsUniform must be a multiple of 16 bytes for WebGL (got {} bytes)",
            size_of::<PlotPointsUniform>()
        );
    }

    #[test]
    fn max_plot_points_constant() {
        // MAX_PLOT_POINTS must match the array size in the WESL shader
        // (array<vec4<f32>, 512>). If you change this, update the shader too!
        assert_eq!(MAX_PLOT_POINTS, 512);
    }
}
