// Slug text plugin
//
// Architecture:
//   SlugAtlas (Resource)     - permanent CPU glyph cache + frame atlas compaction
//   SlugTextNode (Component) - a text string to render (content, font, size, color)
//   SlugTextFont (Component) - which registered FontId to use for an entity
//   SlugPlugin               - registers materials, inserts resources, adds systems
//
// System order per tick:
//   1. collect_and_validate_glyphs  - for all changed SlugTextNodes, calls validate_glyphs
//   2. build_frame_atlas_system   - compact visible glyphs; sets frame.dirty
//   3. upload_atlas_system        - if frame.dirty, update SlugMaterial GPU buffers
//   4. sync_text_meshes           - if atlas dirty OR text changed, re-shape + update Mesh

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

use crate::{
    SlugAtlasLayout, SlugMaterial,
    layout::Layout,
    slug::{FontId, SlugAtlas, SlugTextRun},
};

/// Text content and rendering parameters for a slug text entity.
///
/// Positioning and alignment are handled entirely in the shader via `slugtext()`.
/// `SlugTextNode` carries only the intrinsic properties of the text itself -
/// what to render, at what size, and in what color. The *where* and *how* are
/// the shader's concern via `rect` and `layout` at the call site.
#[derive(Component, Clone, Debug)]
pub struct SlugTextNode {
    /// The string to render.
    pub text: String,
    /// Desired font size in pixels via cap-height. Determines the natural scale
    /// of glyph coordinates; the shader may scale further to fit a rect.
    pub font_size: f32,
    /// RGBA8 color applied at the call site in the shader.
    pub color: [u8; 4],
    /// Layout alignment controls how the text is scaled and anchored within
    /// the node rect. Packed into `SlugParams.layout_flags` and read by
    /// `ui_text.wesl` when calling `slugtext()`.
    pub layout: Layout,
}

impl Default for SlugTextNode {
    fn default() -> Self {
        SlugTextNode {
            text: String::new(),
            font_size: 24.0,
            color: [255, 255, 255, 255],
            layout: Layout::default(), // Center + Center
        }
    }
}

/// Which font (by [`FontId`]) this entity should render with.
/// If absent the entity is skipped by the slug systems.
#[derive(Component, Clone, Copy, Debug)]
pub struct SlugTextFont(pub FontId);

/// Bevy plugin that registers all slug text infrastructure.
///
/// After adding this plugin:
/// 1. Call `world.resource_mut::<SlugAtlas>().register_font(bytes)` to register
///    a font and get a `FontId`.
/// 2. Spawn an entity with [`SlugTextNode`] + [`SlugTextFont`] + a
///    `MaterialMeshBundle<SlugMaterial>` or `MaterialNode<SlugMaterial>`.
pub struct SlugPlugin;

impl Plugin for SlugPlugin {
    fn build(&self, app: &mut App) {
        // Register material for both 2D mesh and UI node rendering.
        app.add_plugins(Material2dPlugin::<SlugMaterial>::default());
        app.add_plugins(UiMaterialPlugin::<SlugMaterial>::default());
        // TODO: 3d extrusion so I can reduce my binary size and dep counts

        // Insert the combined atlas resource.
        app.insert_resource(SlugAtlas::default());

        // Tracks whether the frame atlas changed this cycle.
        app.insert_resource(AtlasDirtyFlag(false));

        app.add_systems(
            Update,
            (
                collect_and_validate_glyphs,
                build_frame_atlas_system,
                upload_atlas_system,
                sync_text_meshes,
            )
                .chain(),
        );
        app.add_systems(
            bevy::app::PostUpdate,
            sync_node_size.after(bevy::ui::UiSystems::Layout),
        );
    }
}

/// Set to true when build_frame_atlas returns true; cleared after sync_text_meshes.
#[derive(Resource, Default)]
struct AtlasDirtyFlag(bool);

/// For every changed SlugTextNode, call validate_glyphs so the CPU cache is warm.
fn collect_and_validate_glyphs(
    mut atlas: ResMut<SlugAtlas>,
    changed_q: Query<(&SlugTextNode, &SlugTextFont), Changed<SlugTextNode>>,
) {
    for (node, font) in changed_q.iter() {
        let result = atlas.validate_glyphs(font.0, &node.text);
        if !result.missing.is_empty() {
            bevy::log::warn!(
                "SlugAtlas: font {:?} missing outlines for: {:?}",
                font.0,
                result.missing
            );
        }
    }
}

/// Compact the visible glyph set into GPU buffers. Runs after validate_glyphs.
fn build_frame_atlas_system(
    mut atlas: ResMut<SlugAtlas>,
    all_q: Query<(&SlugTextNode, &SlugTextFont)>,
    mut flag: ResMut<AtlasDirtyFlag>,
) {
    // Collect (font_id, glyph_ids) for every visible text node.
    let mut per_font: std::collections::HashMap<FontId, Vec<u16>> =
        std::collections::HashMap::new();

    for (node, font) in all_q.iter() {
        let ids = atlas.collect_glyph_ids(font.0, &node.text);
        per_font.entry(font.0).or_default().extend(ids);
    }

    let needed: Vec<(FontId, Vec<u16>)> = per_font.into_iter().collect();
    let rebuilt = atlas.build_frame_atlas(&needed);
    flag.0 = rebuilt;
}

/// If the atlas was rebuilt, upload new GPU buffers into every SlugMaterial.
#[cfg(not(feature = "webgl"))]
fn upload_atlas_system(
    flag: Res<AtlasDirtyFlag>,
    atlas: Res<SlugAtlas>,
    mat_q: Query<&MaterialNode<SlugMaterial>>,
    mat2d_q: Query<&MeshMaterial2d<SlugMaterial>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    if !flag.0 {
        return;
    }

    let layout = SlugAtlasLayout {
        curves_data: atlas.frame.curves.clone(),
        curve_indices_data: atlas.frame.curve_indices.clone(),
        glyphs_data: atlas.frame.glyphs.clone(),
    };

    let all_handles: Vec<_> = mat_q
        .iter()
        .map(|mn| mn.0.clone())
        .chain(mat2d_q.iter().map(|mm| mm.0.clone()))
        .collect();

    for handle in all_handles {
        let Some(mat) = materials.get_mut(&handle) else {
            continue;
        };
        upload_layout_native(mat, &layout, &mut buffers);
    }
}

/// webgl only upload atlas data textures.
#[cfg(feature = "webgl")]
fn upload_atlas_system(
    flag: Res<AtlasDirtyFlag>,
    atlas: Res<SlugAtlas>,
    mat_q: Query<&MaterialNode<SlugMaterial>>,
    mat2d_q: Query<&MeshMaterial2d<SlugMaterial>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if !flag.0 {
        return;
    }

    let layout = SlugAtlasLayout {
        curves_data: atlas.frame.curves.clone(),
        curve_indices_data: atlas.frame.curve_indices.clone(),
        glyphs_data: atlas.frame.glyphs.clone(),
    };

    layout.assert_fits_webgl_textures();

    let all_handles: Vec<_> = mat_q
        .iter()
        .map(|mn| mn.0.clone())
        .chain(mat2d_q.iter().map(|mm| mm.0.clone()))
        .collect();

    for handle in all_handles {
        let Some(mat) = materials.get_mut(&handle) else {
            continue;
        };
        mat.curves_image = images.add(layout.curves_image());
        mat.curve_indices_image = images.add(layout.curve_indices_image());
        mat.glyphs_image = images.add(layout.glyphs_image());
    }
}

#[cfg(not(feature = "webgl"))]
fn upload_layout_native(
    mat: &mut SlugMaterial,
    layout: &SlugAtlasLayout,
    buffers: &mut ResMut<Assets<ShaderStorageBuffer>>,
) {
    // minStorageBufferOffsetAlignment is 256 for now as a safe ish default.
    // The sub-range bindings in AsBindGroup::as_bind_group rely on these
    // offsets being aligned to this boundary.
    const ALIGN: usize = 256;
    fn align_up(v: usize) -> usize {
        (v + ALIGN - 1) & !(ALIGN - 1)
    }

    let c_off: usize = 0;
    let c_sz: usize = layout.curves_data.len();
    let ci_off: usize = align_up(c_sz);
    let ci_sz: usize = layout.curve_indices_data.len();
    let g_off: usize = align_up(ci_off + ci_sz);
    let g_sz: usize = layout.glyphs_data.len();
    let total: usize = g_off + g_sz;

    let mut packed = vec![0u8; total];
    packed[c_off..c_off + c_sz].copy_from_slice(&layout.curves_data);
    packed[ci_off..ci_off + ci_sz].copy_from_slice(&layout.curve_indices_data);
    packed[g_off..g_off + g_sz].copy_from_slice(&layout.glyphs_data);

    use bevy::asset::RenderAssetUsages;
    let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;

    if let Some(ref handle) = mat.atlas_buffer
        && let Some(buf) = buffers.get_mut(handle)
    {
        *buf = ShaderStorageBuffer::new(&packed, usage);
        mat.curves_offset = c_off as u64;
        mat.curves_size = c_sz as u64;
        mat.curve_indices_offset = ci_off as u64;
        mat.curve_indices_size = ci_sz as u64;
        mat.glyphs_offset = g_off as u64;
        mat.glyphs_size = g_sz as u64;
        return;
    }
    let handle = buffers.add(ShaderStorageBuffer::new(&packed, usage));
    mat.atlas_buffer = Some(handle);
    mat.curves_offset = c_off as u64;
    mat.curves_size = c_sz as u64;
    mat.curve_indices_offset = ci_off as u64;
    mat.curve_indices_size = ci_sz as u64;
    mat.glyphs_offset = g_off as u64;
    mat.glyphs_size = g_sz as u64;
}

/// Re-shape and upload vertex/index buffers when text or atlas changes.
#[cfg(not(feature = "webgl"))]
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn sync_text_meshes(
    atlas: Res<SlugAtlas>,
    flag: Res<AtlasDirtyFlag>,
    changed_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugMaterial>>,
    )>,
    mesh_q: Query<&bevy::prelude::Mesh2d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut commands: Commands,
) {
    // If atlas was rebuilt, every entity's mesh is stale regardless of text changes.
    let iter: Box<
        dyn Iterator<
            Item = (
                Entity,
                &SlugTextNode,
                &SlugTextFont,
                Option<&MaterialNode<SlugMaterial>>,
            ),
        >,
    > = if flag.0 {
        Box::new(all_q.iter())
    } else {
        Box::new(changed_q.iter())
    };

    for (entity, node, font, mat_node) in iter {
        let Some(run) = atlas.shape(font.0, &node.text, node.font_size, node.color) else {
            bevy::log::warn!("SlugAtlas::shape failed for entity {:?}", entity);
            continue;
        };

        if run.is_empty() {
            continue;
        }

        // Upload SlugDrawData (runs + glyph_layout) to the UiMaterial.
        if let Some(mn) = mat_node
            && let Some(mat) = materials.get_mut(mn)
        {
            use crate::SlugRunDesc;
            use bevy::asset::RenderAssetUsages;

            let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;

            // text_color and layout_flags live on the material/params, not in the lib.
            let [r, g, b, a] = node.color;
            mat.text_color = bevy::math::Vec4::new(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            );
            mat.params.layout_flags = node.layout.to_u32();

            // Build one SlugRunDesc for this single-string entity.
            let run_desc = SlugRunDesc {
                natural_advance: run.natural_advance,
                natural_height: run.natural_height,
                glyph_offset: 0,
                glyph_count: run.glyph_layout.len() as u32,
            };

            // Pack SlugDrawData: [runs section][pad to 256][glyph_layout section]
            const ALIGN: usize = 256;
            fn align_up(v: usize) -> usize {
                (v + ALIGN - 1) & !(ALIGN - 1)
            }

            let runs_bytes: &[u8] = bytemuck::bytes_of(&run_desc);
            let layout_bytes: &[u8] = bytemuck::cast_slice(&run.glyph_layout);
            let layout_off = align_up(runs_bytes.len());
            let total = layout_off + layout_bytes.len();

            let mut packed = vec![0u8; total];
            packed[..runs_bytes.len()].copy_from_slice(runs_bytes);
            packed[layout_off..layout_off + layout_bytes.len()].copy_from_slice(layout_bytes);

            mat.runs_offset = 0;
            mat.runs_size = runs_bytes.len() as u64;
            mat.glyph_layout_offset = layout_off as u64;
            mat.glyph_layout_size = layout_bytes.len() as u64;

            if let Some(ref handle) = mat.draw_buffer {
                if let Some(buf) = buffers.get_mut(handle) {
                    *buf = ShaderStorageBuffer::new(&packed, usage);
                } else {
                    mat.draw_buffer = Some(buffers.add(ShaderStorageBuffer::new(&packed, usage)));
                }
            } else {
                mat.draw_buffer = Some(buffers.add(ShaderStorageBuffer::new(&packed, usage)));
            }
        }

        // Upload vertex/index mesh for the Material2d path.
        let mesh = build_mesh_from_run(&run);

        if let Ok(existing) = mesh_q.get(entity)
            && let Some(m) = meshes.get_mut(&existing.0)
        {
            *m = mesh;
            continue;
        }

        let handle = meshes.add(mesh);
        commands
            .entity(entity)
            .insert(bevy::prelude::Mesh2d(handle));
    }
}

/// Webgl version of sync_text_meshes.
#[cfg(feature = "webgl")]
fn sync_text_meshes(
    atlas: Res<SlugAtlas>,
    flag: Res<AtlasDirtyFlag>,
    changed_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugMaterial>>,
    )>,
    mesh_q: Query<&bevy::prelude::Mesh2d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let iter: Box<
        dyn Iterator<
            Item = (
                Entity,
                &SlugTextNode,
                &SlugTextFont,
                Option<&MaterialNode<SlugMaterial>>,
            ),
        >,
    > = if flag.0 {
        Box::new(all_q.iter())
    } else {
        Box::new(changed_q.iter())
    };

    for (entity, node, font, mat_node) in iter {
        let Some(run) = atlas.shape(font.0, &node.text, node.font_size, node.color) else {
            continue;
        };

        if run.is_empty() {
            continue;
        }

        if let Some(mn) = mat_node {
            if let Some(mat) = materials.get_mut(mn) {
                use bevy::asset::RenderAssetUsages;
                use bevy::prelude::Image;
                use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

                let width = crate::SLUG_TEX_WIDTH;
                let usage = RenderAssetUsages::RENDER_WORLD;

                // text_color and layout_flags on the material/params, not in the lib.
                let [r, g, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.params.layout_flags = node.layout.to_u32();

                // runs texture: rgba32float, 1 texel per run.
                // Store glyph_offset and glyph_count as IEEE 754 f32
                // values so the shader can recover
                // them via u32(t.z) / u32(t.w) without weird af denormal-flush issues.
                // Exact for all integers up to 2^24 (~16M), should be enough
                // for current and future use cases... (? I hope if not then I
                // gotta do double f32 to avoid the 754 upper 8 bits)
                let runs_data: [f32; 4] = [
                    run.natural_advance,
                    run.natural_height,
                    0f32,
                    run.glyph_layout.len() as f32,
                ];
                mat.runs_image = images.add(Image::new(
                    Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    bytemuck::cast_slice(&runs_data).to_vec(),
                    TextureFormat::Rgba32Float,
                    usage,
                ));

                // glyph_layout texture: rgba32float, 3 texels per SlugGlyphLayout.
                // screen_rect and em_rect are already f32 - copy directly.
                // glyph_index is u32 - store as f32(glyph_index) so the shader
                // can read it back with u32(t2.x) without denormal-flush hazards.
                let count = run.glyph_layout.len();
                let texels = count * 3;
                let height = ((texels as u32).div_ceil(width)).max(1);
                let total_floats = (width * height) as usize * 4;
                let mut px: Vec<f32> = vec![0.0f32; total_floats];
                for (gi, g) in run.glyph_layout.iter().enumerate() {
                    let base = gi * 3 * 4;
                    // texel 0: screen_rect
                    px[base..base + 4].copy_from_slice(&g.screen_rect);
                    // texel 1: em_rect
                    px[base + 4..base + 8].copy_from_slice(&g.em_rect);
                    // texel 2: glyph_index as proper f32 (channels 1-3 stay 0.0)
                    px[base + 8] = g.glyph_index as f32;
                }
                mat.glyph_layout_image = images.add(Image::new(
                    Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    bytemuck::cast_slice(&px).to_vec(),
                    TextureFormat::Rgba32Float,
                    usage,
                ));
            }
        }

        let mesh = build_mesh_from_run(&run);
        if let Ok(existing) = mesh_q.get(entity) {
            if let Some(m) = meshes.get_mut(&existing.0) {
                *m = mesh;
                continue;
            }
        }
        let handle = meshes.add(mesh);
        commands
            .entity(entity)
            .insert(bevy::prelude::Mesh2d(handle));
    }
}

/// Keep `SlugMaterial::params.node_size` in sync with the resolved UI node size.
fn sync_node_size(
    q: Query<(&bevy::ui::ComputedNode, &MaterialNode<SlugMaterial>)>,
    mut materials: ResMut<Assets<SlugMaterial>>,
) {
    for (computed, mat_node) in &q {
        let size = computed.size();
        if size.x > 0.0
            && size.y > 0.0
            && let Some(mat) = materials.get_mut(mat_node)
            && (mat.params.node_size - size).length_squared() > 1e-4
        {
            mat.params.node_size = size;
        }
    }
}

/// Build a `Mesh` from a `SlugTextRun` using the 28-byte slug vertex layout.
///
/// Attribute layout (matches vertex shader `@location` bindings in slug/text.wesl):
///   @location(0) "slug_pos"   Vec4  - screen-space xy + corner sign xy
///   @location(1) "slug_glyph" UVec2 - [packed em coords, glyph_index]
///   @location(2) "slug_color" Vec4  - RGBA8 unpacked to Vec4
fn build_mesh_from_run(run: &SlugTextRun) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues};
    use bevy::render::render_resource::VertexFormat;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );

    let pos_attr = MeshVertexAttribute::new("slug_pos", 0, VertexFormat::Float32x4);
    let glyph_attr = MeshVertexAttribute::new("slug_glyph", 1, VertexFormat::Uint32x2);
    let color_attr = MeshVertexAttribute::new("slug_color", 2, VertexFormat::Float32x4);

    let mut positions: Vec<[f32; 4]> = Vec::with_capacity(run.vertices.len());
    let mut glyphs: Vec<[u32; 2]> = Vec::with_capacity(run.vertices.len());
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(run.vertices.len());

    for v in &run.vertices {
        positions.push(v.pos);
        glyphs.push(v.glyph);
        let [r, g, b, a] = v.color;
        colors.push([
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]);
    }

    mesh.insert_attribute(pos_attr, VertexAttributeValues::Float32x4(positions));
    mesh.insert_attribute(glyph_attr, VertexAttributeValues::Uint32x2(glyphs));
    mesh.insert_attribute(color_attr, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(run.indices.clone()));

    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRA_TTF: &[u8] = include_bytes!("../tests/fixtures/FiraMono-Medium.ttf");

    #[test]
    fn slug_atlas_register_and_ensure() {
        let mut atlas = SlugAtlas::default();
        let fid = atlas.register_font(FIRA_TTF.to_vec()).unwrap();
        let r = atlas.validate_glyphs(fid, "hello");
        assert!(r.atlas_grew(), "first ensure must add glyphs");
        assert!(r.missing.is_empty());
    }

    #[test]
    fn slug_atlas_shape_after_frame_build() {
        let mut atlas = SlugAtlas::default();
        let fid = atlas.register_font(FIRA_TTF.to_vec()).unwrap();
        atlas.validate_glyphs(fid, "Hi");
        let ids = atlas.collect_glyph_ids(fid, "Hi");
        atlas.build_frame_atlas(&[(fid, ids)]);

        let run = atlas
            .shape(fid, "Hi", 32.0, [255; 4])
            .expect("shape must succeed");
        assert!(!run.is_empty());
        assert_eq!(run.vertices.len(), 8, "2 glyphs x 4 verts");
        assert_eq!(run.indices.len(), 12, "2 glyphs x 6 indices");
    }

    #[test]
    fn slug_atlas_layout_assert_fits_small_charset() {
        let mut atlas = SlugAtlas::default();
        let fid = atlas.register_font(FIRA_TTF.to_vec()).unwrap();
        atlas.validate_glyphs(fid, "fps 0123456789.");
        let ids = atlas.collect_glyph_ids(fid, "fps 0123456789.");
        atlas.build_frame_atlas(&[(fid, ids)]);

        let layout = SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };
        // Should not panic for this small charset.
        layout.assert_fits_webgl_textures();
    }
}
