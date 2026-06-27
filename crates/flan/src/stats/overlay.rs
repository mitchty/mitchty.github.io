//! Stats overlay material types, plugin, and unified handle.
//!
//! Two `UiMaterial` variants for now. Callers select the appropriate material
//! via [`ShaderVariant`] and store the result as the [`StatsOverlayHandle`]
//! resource - callers never need to branch on which concrete material type is
//! active.
//!
//! | Type                            | Data path       | Target            |
//! |---------------------------------|-----------------|-------------------|
//! | [`StatsOverlayMaterial`]        | Storage buffers | native / WebGPU   |
//! | [`StatsOverlayTextureMaterial`] | rgba32 textures | WebGL2            |
//!
//! Register [`StatsOverlayMaterialPlugin`] **after** `ShadersPlugin`.
// TODO: all the above is why this all needs to be one stupid plugin so api
// users don't need to give a crap about all this plumbing.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, AsBindGroupError, BindGroupEntries, BindGroupLayoutEntry, BindingResources,
    BindingType, BufferBindingType, Extent3d, PreparedBindGroup, ShaderStages, ShaderType,
    TextureDimension, TextureFormat, TextureSampleType, TextureViewDimension,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bytemuck::cast_slice;

use crate::ShaderVariant;
use crate::shaders::{stats_overlay_default_shader_handle, stats_overlay_texture_shader_handle};

/// CPU-side mirror of `StatsOverlayUniform`.
///
/// ```text
/// offset  0  node_size         vec2<f32>
/// offset  8  min_fps           f32
/// offset 12  max_fps           f32
/// offset 16  line_width        f32
/// offset 20  layout_flags      u32
/// offset 24  alpha_discard     f32
/// offset 28  _pad              f32
/// offset 32  text_color        vec4<f32>
/// offset 48  background_color  vec4<f32>
/// total = 64 bytes
/// ```
#[derive(Clone, ShaderType)]
pub struct StatsOverlayParams {
    pub node_size: Vec2,
    pub min_fps: f32,
    pub max_fps: f32,
    pub line_width: f32,
    pub layout_flags: u32,
    pub alpha_discard: f32,
    pub _pad: f32,
    pub text_color: Vec4,
    pub background_color: Vec4,
}

impl Default for StatsOverlayParams {
    fn default() -> Self {
        Self {
            node_size: Vec2::new(230.0, 40.0),
            min_fps: 0.0,
            max_fps: 120.0,
            line_width: 0.01,
            layout_flags: 0x08, // SLUG_LAYOUT_RIGHT | VCENTER
            alpha_discard: 0.01,
            _pad: 0.0,
            text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            background_color: Vec4::new(1.0, 1.0, 1.0, 0.0),
        }
    }
}

/// Stats overlay `UiMaterial`.
///
/// Bindings at `@group(1)`:
/// ```text
/// 0  uniform   StatsOverlayParams        (64bytes)
/// 1  storage   fps_points                RuntimeArray<f32>
/// 2  storage   curves                    RuntimeArray<[vec2;3]>
/// 3  storage   curve_indices             RuntimeArray<u32>
/// 4  storage   glyphs                    RuntimeArray<GlyphInfo>
/// 5  storage   runs                      RuntimeArray<SlugRunDesc>
/// 6  storage   glyph_layout              RuntimeArray<SlugGlyphLayout>
/// ```
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct StatsOverlayMaterial {
    #[uniform(0)]
    pub params: StatsOverlayParams,
    #[storage(1, read_only)]
    pub fps_points: Handle<ShaderBuffer>,
    #[storage(2, read_only)]
    pub curves: Handle<ShaderBuffer>,
    #[storage(3, read_only)]
    pub curve_indices: Handle<ShaderBuffer>,
    #[storage(4, read_only)]
    pub glyphs: Handle<ShaderBuffer>,
    #[storage(5, read_only)]
    pub runs: Handle<ShaderBuffer>,
    #[storage(6, read_only)]
    pub glyph_layout: Handle<ShaderBuffer>,
}

impl UiMaterial for StatsOverlayMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(stats_overlay_default_shader_handle())
    }
}

/// Stats overlay `UiMaterial`.
///
/// Bindings at `@group(1)`:
/// ```text
/// 0  uniform   StatsOverlayParams        (64bytes)
/// 1  texture   fps_points_image          rgba32float  64x1
/// 2  texture   curves_image              rgba32float  2 texels/curve
/// 3  texture   curve_indices_image       rgba32uint   4 u32/texel
/// 4  texture   glyphs_image              rgba32uint   5 texels/GlyphInfo
/// 5  texture   runs_image                rgba32float  1 texel/SlugRunDesc
/// 6  texture   glyph_layout_image        rgba32float  3 texels/SlugGlyphLayout
/// ```
///
/// Uses a manual `AsBindGroup` because the derive macro defaults all
/// `#[texture]` bindings to `Float { filterable: true }`, but bindings 3 and
/// 4 are `rgba32uint` (Uint sample type) the derive would produce an invalid
/// layout that wgpu rejects with a validation error at runtime. Need to brain this more
#[derive(Asset, TypePath, Clone)]
pub struct StatsOverlayTextureMaterial {
    pub params: StatsOverlayParams,
    pub fps_points_image: Handle<Image>,
    pub curves_image: Handle<Image>,
    pub curve_indices_image: Handle<Image>,
    pub glyphs_image: Handle<Image>,
    pub runs_image: Handle<Image>,
    pub glyph_layout_image: Handle<Image>,
}

impl AsBindGroup for StatsOverlayTextureMaterial {
    type Data = ();
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "stats_overlay_texture_material"
    }
    fn bind_group_data(&self) {}

    fn as_bind_group(
        &self,
        layout: &bevy::render::render_resource::BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        (images, _fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        use bevy::render::render_resource::ShaderSize;

        // Pack the params uniform using the ShaderType and ShaderSize derive.
        let params_size = StatsOverlayParams::SHADER_SIZE.get() as usize;
        let mut params_bytes = vec![0u8; params_size];
        {
            let mut writer = bevy::render::render_resource::encase::StorageBuffer::new(
                params_bytes.as_mut_slice(),
            );
            writer
                .write(&self.params)
                .expect("StatsOverlayParams encase write must not fail");
        }
        use bevy::render::render_resource::{BufferInitDescriptor, BufferUsages};
        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("stats_overlay_texture_params"),
            contents: &params_bytes,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let get_view = |handle: &Handle<Image>| {
            if handle.id() == bevy::asset::AssetId::default() {
                return Err(AsBindGroupError::RetryNextUpdate);
            }
            images
                .get(handle)
                .map(|g| &g.texture_view)
                .ok_or(AsBindGroupError::RetryNextUpdate)
        };

        let fps_v = get_view(&self.fps_points_image)?;
        let curves_v = get_view(&self.curves_image)?;
        let ci_v = get_view(&self.curve_indices_image)?;
        let glyphs_v = get_view(&self.glyphs_image)?;
        let runs_v = get_view(&self.runs_image)?;
        let layout_v = get_view(&self.glyph_layout_image)?;

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &BindGroupEntries::sequential((
                params_buf.as_entire_binding(),
                fps_v,
                curves_v,
                ci_v,
                glyphs_v,
                runs_v,
                layout_v,
            )),
        );
        Ok(PreparedBindGroup {
            bindings: BindingResources(vec![]),
            bind_group,
        })
    }

    fn unprepared_bind_group(
        &self,
        _layout: &bevy::render::render_resource::BindGroupLayout,
        _render_device: &bevy::render::renderer::RenderDevice,
        _param: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<bevy::render::render_resource::UnpreparedBindGroup, AsBindGroupError> {
        Err(AsBindGroupError::CreateBindGroupDirectly)
    }

    fn bind_group_layout_entries(
        _: &bevy::render::renderer::RenderDevice,
        _: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let float_tex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uint_tex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Uint,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            float_tex(1), // fps_points_image    rgba32float
            float_tex(2), // curves_image        rgba32float
            uint_tex(3),  // curve_indices_image rgba32uint
            uint_tex(4),  // glyphs_image        rgba32uint
            float_tex(5), // runs_image          rgba32float
            float_tex(6), // glyph_layout_image  rgba32float
        ]
    }
}

impl UiMaterial for StatsOverlayTextureMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(stats_overlay_texture_shader_handle())
    }
}

/// Typed handle to whichever stats overlay material variant was created.
///
/// Store this as a Bevy [`Resource`] after construction; all callers work
/// through this type without needing to know which concrete material is active.
///
/// ```no_run
/// // update params without branching on the variant:
/// handle.set_params(params, &mut materials, &mut texture_materials);
/// ```
#[derive(Resource, Clone)]
pub enum StatsOverlayHandle {
    /// Storage-buffer path - [`StatsOverlayMaterial`].
    Default(Handle<StatsOverlayMaterial>),
    /// Texture path - [`StatsOverlayTextureMaterial`].
    Texture(Handle<StatsOverlayTextureMaterial>),
}

impl StatsOverlayHandle {
    /// Which GPU data path is active.
    pub fn variant(&self) -> ShaderVariant {
        match self {
            Self::Default(_) => ShaderVariant::Default,
            Self::Texture(_) => ShaderVariant::Texture,
        }
    }

    /// Update the material's uniform params without the caller needing to
    /// know which concrete material type is stored.
    pub fn set_params(
        &self,
        params: StatsOverlayParams,
        materials: &mut Assets<StatsOverlayMaterial>,
        texture_materials: &mut Assets<StatsOverlayTextureMaterial>,
    ) {
        match self {
            Self::Default(h) => {
                if let Some(mut mat) = materials.get_mut(h) {
                    mat.params = params;
                }
            }
            Self::Texture(h) => {
                if let Some(mut mat) = texture_materials.get_mut(h) {
                    mat.params = params;
                }
            }
        }
    }
}

/// Build a `64x1 rgba32float` image from 256 averaged FPS values.
///
/// Four `f32` values are packed per texel so the shader reads:
/// ```wgsl
/// let t = textureLoad(FPS_POINTS_TEX, vec2i(i / 4, 0), 0);
/// let v = t[i % 4];
/// ```
pub fn build_fps_points_image(data: &[f32; 256]) -> Image {
    // 256 values -> 64 texels x 4 channels = 256 floats total.
    let mut floats = vec![0.0f32; 64 * 4];
    for (i, &v) in data.iter().enumerate() {
        floats[i] = v;
    }
    Image::new(
        Extent3d {
            width: 64,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        cast_slice::<f32, u8>(&floats).to_vec(),
        TextureFormat::Rgba32Float,
        // MAIN_WORLD | RENDER_WORLD: Bevy keeps a CPU copy for re-upload
        // after a pipeline reset same convention as build_runs_image for now.
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Registers both stats overlay `UiMaterial` pipelines.
///
/// Add after `ShadersPlugin`, then create whichever material type fits the
/// target platform and wrap it in [`StatsOverlayHandle`].
pub struct StatsOverlayMaterialPlugin;

impl Plugin for StatsOverlayMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<StatsOverlayMaterial>::default());
        app.add_plugins(UiMaterialPlugin::<StatsOverlayTextureMaterial>::default());
    }
}
