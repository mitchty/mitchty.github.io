//! Slug text material types and plugin for `flan::slug::text` and
//! `flan::slug::text3d` shaders.
//!
//! Four material types, all always compiled for now later feature gated:
//!
//! | Type                       | Pipeline   | Data path       | Group |
//! |----------------------------|------------|-----------------|-------|
//! | [`SlugTextMaterial`]       | UiMaterial | storage buffers | 1     |
//! | [`SlugTextTextureMaterial`]| UiMaterial | rgba32 textures | 1     |
//! | [`SlugText3dMaterial`]     | Material3d | storage buffers | 3     |
//! | [`SlugText3dTextureMaterial`]| Material3d| rgba32 textures | 3    |
//!
//! Both uniform-binding paths pack the same 96-byte layout:
//! ```text
//!   offset  0  node_size      vec2<f32>   (16bytes via SlugParams)
//!   offset 16  text_color     vec4<f32>
//!   offset 32  local_to_clip  mat4x4<f32>
//! ```
//!
//! Register [`SlugTextMaterialPlugin`] **after** `ShadersPlugin`.

use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::render::alpha::AlphaMode;
#[cfg(not(feature = "webgl"))]
use bevy::render::render_resource::BindGroupEntry;
use bevy::render::render_resource::{
    AsBindGroup, AsBindGroupError, BindGroupEntries, BindGroupLayoutEntry, BindingResources,
    BindingType, BufferBindingType, BufferInitDescriptor, BufferUsages, Face, PreparedBindGroup,
    RenderPipelineDescriptor, ShaderStages, SpecializedMeshPipelineError, TextureSampleType,
    TextureViewDimension,
};
use bevy::shader::ShaderRef;

use crate::SlugParams;
#[cfg(not(feature = "webgl"))]
use crate::shaders::{slug_text_default_shader_handle, slug_text3d_default_shader_handle};
use crate::shaders::{slug_text_texture_shader_handle, slug_text3d_texture_shader_handle};

/// Pack `SlugParams` (16bytes) + `text_color` (16bytes) + `local_to_clip` (64bytes)
/// into a 96-byte array that matches `SlugParamsUniform` in the WGSL shaders.
fn pack_params(params: &SlugParams, text_color: Vec4, local_to_clip: &[[f32; 4]; 4]) -> [u8; 96] {
    let mut buf = [0u8; 96];
    buf[..16].copy_from_slice(bytemuck::bytes_of(params));
    buf[16..32].copy_from_slice(bytemuck::bytes_of(&text_color));
    buf[32..96].copy_from_slice(bytemuck::cast_slice(local_to_clip));
    buf
}

/// Slug text `UiMaterial`.
///
/// Bindings at `@group(1)`:
/// ```text
/// 0  uniform  SlugParamsUniform (96bytes)
/// 1  storage  curves[]          RuntimeArray<[vec2f; 3]>
/// 2  storage  curve_indices[]   RuntimeArray<u32>
/// 3  storage  glyphs[]          RuntimeArray<GlyphInfo>
/// 4  storage  runs[]            RuntimeArray<SlugRunDesc>
/// 5  storage  glyph_layout[]    RuntimeArray<SlugGlyphLayout>
/// ```
#[cfg(not(feature = "webgl"))]
#[derive(Asset, TypePath, Clone)]
pub struct SlugTextMaterial {
    pub params: SlugParams,
    pub text_color: Vec4,
    pub local_to_clip: [[f32; 4]; 4],
    pub curves: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub curve_indices: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub glyphs: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub runs: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub glyph_layout: Handle<bevy::render::storage::ShaderStorageBuffer>,
}

#[cfg(not(feature = "webgl"))]
impl AsBindGroup for SlugTextMaterial {
    type Data = ();
    type Param = bevy::ecs::system::lifetimeless::SRes<
        bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderStorageBuffer>,
    >;

    fn label() -> &'static str {
        "slug_text_material"
    }
    fn bind_group_data(&self) {}

    fn as_bind_group(
        &self,
        layout: &bevy::render::render_resource::BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        gpu_buffers: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let get = |h: &Handle<bevy::render::storage::ShaderStorageBuffer>| {
            gpu_buffers.get(h).ok_or(AsBindGroupError::RetryNextUpdate)
        };
        let curves_gpu = get(&self.curves)?;
        let ci_gpu = get(&self.curve_indices)?;
        let glyphs_gpu = get(&self.glyphs)?;
        let runs_gpu = get(&self.runs)?;
        let layout_gpu = get(&self.glyph_layout)?;

        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("slug_text_params"),
            contents: &pack_params(&self.params, self.text_color, &self.local_to_clip),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: curves_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: ci_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: glyphs_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: runs_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: layout_gpu.buffer.as_entire_binding(),
                },
            ],
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
        let ssb = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            ssb(1),
            ssb(2),
            ssb(3),
            ssb(4),
            ssb(5),
        ]
    }
}

#[cfg(not(feature = "webgl"))]
impl Default for SlugTextMaterial {
    fn default() -> Self {
        Self {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            curves: Handle::default(),
            curve_indices: Handle::default(),
            glyphs: Handle::default(),
            runs: Handle::default(),
            glyph_layout: Handle::default(),
        }
    }
}

#[cfg(not(feature = "webgl"))]
impl UiMaterial for SlugTextMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text_default_shader_handle())
    }
}

/// Slug text `UiMaterial`.
///
/// Bindings at `@group(1)`:
/// ```text
/// 0  uniform  SlugParamsUniform  (96bytes)
/// 1  texture  curves_tex         rgba32float  2 texels/curve
/// 2  texture  curve_indices_tex  rgba32uint   4 u32/texel
/// 3  texture  glyphs_tex         rgba32uint   5 texels/GlyphInfo
/// 4  texture  runs_tex           rgba32float  1 texel/SlugRunDesc
/// 5  texture  glyph_layout_tex   rgba32float  3 texels/SlugGlyphLayout
/// ```
#[derive(Asset, TypePath, Clone)]
pub struct SlugTextTextureMaterial {
    pub params: SlugParams,
    pub text_color: Vec4,
    pub local_to_clip: [[f32; 4]; 4],
    pub curves_image: Handle<Image>,
    pub curve_indices_image: Handle<Image>,
    pub glyphs_image: Handle<Image>,
    pub runs_image: Handle<Image>,
    pub glyph_layout_image: Handle<Image>,
}

impl Default for SlugTextTextureMaterial {
    fn default() -> Self {
        Self {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            curves_image: Handle::default(),
            curve_indices_image: Handle::default(),
            glyphs_image: Handle::default(),
            runs_image: Handle::default(),
            glyph_layout_image: Handle::default(),
        }
    }
}

impl AsBindGroup for SlugTextTextureMaterial {
    type Data = ();
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "slug_text_texture_material"
    }
    fn bind_group_data(&self) {}

    fn as_bind_group(
        &self,
        layout: &bevy::render::render_resource::BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        (images, _fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let get_view = |handle: &Handle<Image>| {
            if handle.id() == bevy::asset::AssetId::default() {
                return Err(AsBindGroupError::RetryNextUpdate);
            }
            images
                .get(handle)
                .map(|g| &g.texture_view)
                .ok_or(AsBindGroupError::RetryNextUpdate)
        };
        let curves_v = get_view(&self.curves_image)?;
        let ci_v = get_view(&self.curve_indices_image)?;
        let glyphs_v = get_view(&self.glyphs_image)?;
        let runs_v = get_view(&self.runs_image)?;
        let layout_v = get_view(&self.glyph_layout_image)?;

        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("slug_text_texture_params"),
            contents: &pack_params(&self.params, self.text_color, &self.local_to_clip),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &BindGroupEntries::sequential((
                params_buf.as_entire_binding(),
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
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            float_tex(1),
            uint_tex(2),
            uint_tex(3),
            float_tex(4),
            float_tex(5),
        ]
    }
}

impl UiMaterial for SlugTextTextureMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text_texture_shader_handle())
    }
}

/// Slug text `Material` Material3D.
///
/// Atlas data curves, curve_indices, glyphs etc... is shared across all 3D text
/// entities via [`SlugAtlasBuffers`] and all instances point to the same three
/// SSB handles so there is exactly one GPU buffer per atlas section.
/// `params_buf` is pre-allocated per-entity so `as_bind_group` never calls
/// `create_buffer_with_data` on the hot path.
///
/// Bindings at `@group(3)`:
/// ```text
/// 0  uniform  SlugParamsUniform (96bytes)  - pre-alloc UNIFORM | COPY_DST
/// 1  storage  curves[]                  - shared from SlugAtlasBuffers
/// 2  storage  curve_indices[]           - shared from SlugAtlasBuffers
/// 3  storage  glyphs[]                  - shared from SlugAtlasBuffers
/// ```
#[cfg(not(feature = "webgl"))]
#[derive(Asset, TypePath, Clone)]
pub struct SlugText3dMaterial {
    pub params: SlugParams,
    pub text_color: Vec4,
    pub is_extruded: bool,
    pub local_to_clip: [[f32; 4]; 4],
    /// Pre-allocated 96-byte uniform buffer (UNIFORM | COPY_DST).
    /// Allocated once by `init_slug_entity` and written to each frame by the
    /// params upload system. `None` until first allocation and `as_bind_group`
    /// returns `RetryNextUpdate` while this is `None`.
    pub params_buf: Option<Handle<bevy::render::storage::ShaderStorageBuffer>>,
    /// Shared atlas SSBs all three come from [`SlugAtlasBuffers`].
    /// Default handles are replaced by `upload_atlas_system` on first wgpu upload.
    pub curves: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub curve_indices: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub glyphs: Handle<bevy::render::storage::ShaderStorageBuffer>,
}

#[cfg(not(feature = "webgl"))]
impl Default for SlugText3dMaterial {
    fn default() -> Self {
        SlugText3dMaterial {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            is_extruded: false,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            params_buf: None,
            curves: Handle::default(),
            curve_indices: Handle::default(),
            glyphs: Handle::default(),
        }
    }
}

#[cfg(not(feature = "webgl"))]
impl AsBindGroup for SlugText3dMaterial {
    type Data = bool;
    type Param = bevy::ecs::system::lifetimeless::SRes<
        bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderStorageBuffer>,
    >;

    fn label() -> &'static str {
        "slug_text3d_material"
    }
    fn bind_group_data(&self) -> bool {
        self.is_extruded
    }

    fn as_bind_group(
        &self,
        layout: &bevy::render::render_resource::BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        gpu_buffers: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let params_gpu = match &self.params_buf {
            Some(h) => gpu_buffers
                .get(h)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => return Err(AsBindGroupError::RetryNextUpdate),
        };
        let get = |h: &Handle<bevy::render::storage::ShaderStorageBuffer>| {
            if h.id() == bevy::asset::AssetId::default() {
                return Err(AsBindGroupError::RetryNextUpdate);
            }
            gpu_buffers.get(h).ok_or(AsBindGroupError::RetryNextUpdate)
        };
        let curves_gpu = get(&self.curves)?;
        let ci_gpu = get(&self.curve_indices)?;
        let glyphs_gpu = get(&self.glyphs)?;

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: curves_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: ci_gpu.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: glyphs_gpu.buffer.as_entire_binding(),
                },
            ],
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
        let ssb = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            ssb(1),
            ssb(2),
            ssb(3),
        ]
    }
}

#[cfg(not(feature = "webgl"))]
impl Material for SlugText3dMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text3d_default_shader_handle())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text3d_default_shader_handle())
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(ref mut ds) = descriptor.depth_stencil {
            ds.depth_write_enabled = true;
        }
        descriptor.primitive.cull_mode = if key.bind_group_data {
            Some(Face::Back)
        } else {
            None
        };
        Ok(())
    }
}

/// Slug text `Material` Material3D.
///
/// Bindings at `@group(3)`:
/// ```text
/// 0  uniform  SlugParamsUniform  96bytes
/// 1  texture  curves_tex         rgba32float  2 texels/curve
/// 2  texture  curve_indices_tex  rgba32uint   4 u32/texel
/// 3  texture  glyphs_tex         rgba32uint   5 texels/GlyphInfo
/// ```
#[derive(Asset, TypePath, Clone)]
pub struct SlugText3dTextureMaterial {
    pub params: SlugParams,
    pub text_color: Vec4,
    pub is_extruded: bool,
    pub local_to_clip: [[f32; 4]; 4],
    pub curves_image: Handle<Image>,
    pub curve_indices_image: Handle<Image>,
    pub glyphs_image: Handle<Image>,
}

impl Default for SlugText3dTextureMaterial {
    fn default() -> Self {
        Self {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            is_extruded: false,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            curves_image: Handle::default(),
            curve_indices_image: Handle::default(),
            glyphs_image: Handle::default(),
        }
    }
}

impl AsBindGroup for SlugText3dTextureMaterial {
    type Data = bool;
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "slug_text3d_texture_material"
    }
    fn bind_group_data(&self) -> bool {
        self.is_extruded
    }

    fn as_bind_group(
        &self,
        layout: &bevy::render::render_resource::BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        (images, _fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let get_view = |handle: &Handle<Image>| {
            if handle.id() == bevy::asset::AssetId::default() {
                return Err(AsBindGroupError::RetryNextUpdate);
            }
            images
                .get(handle)
                .map(|g| &g.texture_view)
                .ok_or(AsBindGroupError::RetryNextUpdate)
        };
        let curves_v = get_view(&self.curves_image)?;
        let ci_v = get_view(&self.curve_indices_image)?;
        let glyphs_v = get_view(&self.glyphs_image)?;

        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("slug_text3d_texture_params"),
            contents: &pack_params(&self.params, self.text_color, &self.local_to_clip),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &BindGroupEntries::sequential((
                params_buf.as_entire_binding(),
                curves_v,
                ci_v,
                glyphs_v,
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
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uint_tex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
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
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            float_tex(1),
            uint_tex(2),
            uint_tex(3),
        ]
    }
}

impl Material for SlugText3dTextureMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text3d_texture_shader_handle())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(slug_text3d_texture_shader_handle())
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(ref mut ds) = descriptor.depth_stencil {
            ds.depth_write_enabled = true;
        }
        descriptor.primitive.cull_mode = if key.bind_group_data {
            Some(Face::Back)
        } else {
            None
        };
        Ok(())
    }
}

/// Shared GPU atlas buffers for the native storage-buffer path.
///
/// There is exactly one set of atlas SSBs for the entire `SlugPlugin`. Every
/// [`SlugText3dMaterial`] instance points to the same three handles so a single
/// `write_buffer` call in the render world updates the atlas for all 3D text
/// entities. Inserted by [`SlugTextMaterialPlugin`]; populated on first upload
/// by `upload_atlas_system`.
///
/// Capacity fields track the allocated byte capacity of each SSB so the upload
/// system can decide whether to `write_buffer` (fits) or reallocate (overflow).
#[cfg(not(feature = "webgl"))]
#[derive(bevy::prelude::Resource, Default, Clone)]
pub struct SlugAtlasBuffers {
    pub curves: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub curve_indices: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub glyphs: Handle<bevy::render::storage::ShaderStorageBuffer>,
    pub curves_cap: u64,
    pub ci_cap: u64,
    pub glyphs_cap: u64,
}

/// Shared GPU atlas images for the WebGL (texture) path.
///
/// Same rationale as [`SlugAtlasBuffers`]: one set of image handles, all
/// [`SlugText3dTextureMaterial`] instances reference the same three handles.
#[derive(bevy::prelude::Resource, Default, Clone)]
pub struct SlugAtlasImages {
    pub curves: Handle<bevy::prelude::Image>,
    pub curve_indices: Handle<bevy::prelude::Image>,
    pub glyphs: Handle<bevy::prelude::Image>,
}

/// Registers all four slug text and text3d material pipelines.
///
/// Add after `ShadersPlugin`.
pub struct SlugTextMaterialPlugin;

impl Plugin for SlugTextMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<SlugTextTextureMaterial>::default());
        app.add_plugins(MaterialPlugin::<SlugText3dTextureMaterial>::default());
        app.init_resource::<SlugAtlasImages>();

        #[cfg(not(feature = "webgl"))]
        {
            app.add_plugins(UiMaterialPlugin::<SlugTextMaterial>::default());
            app.add_plugins(MaterialPlugin::<SlugText3dMaterial>::default());
            app.init_resource::<SlugAtlasBuffers>();
        }
    }
}
