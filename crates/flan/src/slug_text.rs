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

use bevy::pbr::MaterialPlugin;
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
    /// Optional extrusion depth in normalized world units.
    /// `None` = flat 2-D quad/decal in 3d space.
    /// `Some(depth)` places the front face at `+depth/2` and the back face at
    /// `-depth/2` along the local z axis (y in bevy coord system), and spawns a
    /// side-wall child entity for both slug "caps".
    pub depth: Option<f32>,
}

impl Default for SlugTextNode {
    fn default() -> Self {
        SlugTextNode {
            text: String::new(),
            font_size: 24.0,
            color: [255, 255, 255, 255],
            layout: Layout::default(), // Center + Center
            depth: None,
        }
    }
}

/// Marker component placed on the side-wall child entity spawned when
/// `SlugTextNode::depth` is `Some`. Stores the parent entity so the system can
/// find and despawn the parent/child when depth is removed. I'm not sure this
/// approach is good but I wanted to keep the decal version and "3d" versions
/// literally the same thing and don't know how to boolean unify the caps and
/// sidewalls of the text. So... just treat "3d" entities as a collection of
/// their components like a hack.
#[derive(Component)]
pub struct SlugSideWallOf(pub Entity);

/// Which font (by [`FontId`]) this entity should render with.
/// If absent the entity is skipped by the slug systems.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SlugTextFont(pub FontId);

// TODO: Required components here instead? Tbh I kinda like ye olde bevy
// Bundles. Future mitch figure it out.

/// Bundle for placing slug text as a 3D mesh in world space.
///
/// Mimics bevy_slugtext::TextMesh's approach so the mesh is normalized so the
/// full shape is 1 world unit tall by default. That way you can just use normal
/// Transform's at runtime to change things.
///
/// Spawn this bundle and the [`SlugPlugin`] systems will populate `mesh` and
/// upload atlas data to `material` automatically on the first (and every
/// subsequent changed) frame.
///
/// # Example
/// ```no_run
/// commands.spawn(SlugTextMesh {
///     node: SlugTextNode { text: "Hello".into(), color: [255,255,255,255], ..default() },
///     font: SlugTextFont(my_font_id),
///     material: MeshMaterial3d(materials.add(SlugMaterial::default())),
///     transform: Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::splat(2.0)),
///     ..default()
/// });
/// ```
#[derive(Bundle, Default)]
pub struct SlugTextMesh {
    pub node: SlugTextNode,
    pub font: SlugTextFont,
    pub mesh: Mesh3d,
    pub material: MeshMaterial3d<SlugMaterial>,
    pub transform: Transform,
    pub visibility: Visibility,
}

/// Marker resource present on webgl until the atlas textures have been
/// uploaded for the first time. Avoids non initialized frame update issues in webgl.
#[derive(Resource)]
pub struct SlugAtlasNotReady;

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
        // Register material for UI/2d/3d mesh world-space rendering.
        app.add_plugins(Material2dPlugin::<SlugMaterial>::default());
        app.add_plugins(UiMaterialPlugin::<SlugMaterial>::default());
        app.add_plugins(MaterialPlugin::<SlugMaterial>::default());

        // On webgl, block sync_text_meshes from running until the atlas is ready.
        // The marker is removed by upload_atlas_system on the first successful
        // upload.
        #[cfg(feature = "webgl")]
        app.insert_resource(SlugAtlasNotReady);

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
                sync_text_meshes.run_if(not(resource_exists::<SlugAtlasNotReady>)),
            )
                .chain(),
        );
        app.add_systems(
            bevy::app::PostUpdate,
            sync_node_size.after(bevy::ui::UiSystems::Layout),
        );
        app.add_systems(
            bevy::app::PostUpdate,
            sync_slug_3d_transforms.after(bevy::transform::TransformSystems::Propagate),
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
    mat3d_q: Query<&MeshMaterial3d<SlugMaterial>>,
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
        .chain(mat3d_q.iter().map(|mm| mm.0.clone()))
        .collect();

    for handle in all_handles {
        let Some(mat) = materials.get_mut(&handle) else {
            continue;
        };
        upload_layout_native(mat, &layout, &mut buffers);
    }
}

/// webgl only upload atlas data textures. Removes [`SlugAtlasNotReady`] on
/// first successful upload so [`sync_text_meshes`] can run without issues.
#[cfg(feature = "webgl")]
fn upload_atlas_system(
    flag: Res<AtlasDirtyFlag>,
    atlas: Res<SlugAtlas>,
    mat_q: Query<&MaterialNode<SlugMaterial>>,
    mat2d_q: Query<&MeshMaterial2d<SlugMaterial>>,
    mat3d_q: Query<&MeshMaterial3d<SlugMaterial>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
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
        .chain(mat3d_q.iter().map(|mm| mm.0.clone()))
        .collect();

    for handle in all_handles {
        let Some(mat) = materials.get_mut(&handle) else {
            continue;
        };
        mat.curves_image = images.add(layout.curves_image());
        mat.curve_indices_image = images.add(layout.curve_indices_image());
        mat.glyphs_image = images.add(layout.glyphs_image());
    }

    commands.remove_resource::<SlugAtlasNotReady>();
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
            Option<&MeshMaterial3d<SlugMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugMaterial>>,
        Option<&MeshMaterial3d<SlugMaterial>>,
    )>,
    mesh2d_q: Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: Query<&Mesh3d>,
    side_walls_q: Query<(Entity, &SlugSideWallOf)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
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
                Option<&MeshMaterial3d<SlugMaterial>>,
            ),
        >,
    > = if flag.0 {
        Box::new(all_q.iter())
    } else {
        Box::new(changed_q.iter())
    };

    for (entity, node, font, mat_node, mat3d_node) in iter {
        // Shape initially a large internal font_size andthen normalize down to
        // 1 world unit tall inspired from bevy_slugtexts approach.
        if let Some(mm3d) = mat3d_node {
            const INTERNAL_SIZE: f32 = 1000.0;
            let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                bevy::log::warn!("SlugAtlas::shape (3d) failed for entity {:?}", entity);
                continue;
            };
            if run.is_empty() {
                continue;
            }

            let inv_h = if run.natural_height > 0.0 {
                1.0 / run.natural_height
            } else {
                1.0
            };

            // Build normalized draw data and screen_rect coords scaled by
            // inv_h so they sit in [0, 1] world-unit space.
            let normalized_layout: Vec<crate::slug::SlugGlyphLayout> = run
                .glyph_layout
                .iter()
                .map(|g| crate::slug::SlugGlyphLayout {
                    screen_rect: [
                        g.screen_rect[0] * inv_h,
                        g.screen_rect[1] * inv_h,
                        g.screen_rect[2] * inv_h,
                        g.screen_rect[3] * inv_h,
                    ],
                    em_rect: g.em_rect,
                    glyph_index: g.glyph_index,
                    _pad: g._pad,
                })
                .collect();

            if let Some(mat) = materials.get_mut(mm3d) {
                use crate::SlugRunDesc;
                use bevy::asset::RenderAssetUsages;

                let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;

                let [r, g_c, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.params.layout_flags = node.layout.to_u32();

                let run_desc = SlugRunDesc {
                    natural_advance: run.natural_advance * inv_h,
                    natural_height: 1.0,
                    glyph_offset: 0,
                    glyph_count: normalized_layout.len() as u32,
                };

                const ALIGN: usize = 256;
                fn align_up(v: usize) -> usize {
                    (v + ALIGN - 1) & !(ALIGN - 1)
                }

                let runs_bytes: &[u8] = bytemuck::bytes_of(&run_desc);
                let layout_bytes: &[u8] = bytemuck::cast_slice(&normalized_layout);
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
                        mat.draw_buffer =
                            Some(buffers.add(ShaderStorageBuffer::new(&packed, usage)));
                    }
                } else {
                    mat.draw_buffer = Some(buffers.add(ShaderStorageBuffer::new(&packed, usage)));
                }
            }

            // half_adv must match normalize_run_3d so side-wall cursor coords align.
            let half_adv = run.natural_advance * inv_h * 0.5;

            // Build normalized mesh at pos.xy in world units, with optional depth.
            let norm_run = normalize_run_3d(&run, inv_h);
            let mesh = build_mesh_from_run(&norm_run, node.depth);

            if let Ok(existing) = mesh3d_q.get(entity)
                && let Some(m) = meshes.get_mut(&existing.0)
            {
                *m = mesh;
            } else {
                let handle = meshes.add(mesh);
                commands.entity(entity).insert(Mesh3d(handle));
            }

            // Find any associated side-wall child if any for this entity.
            let existing_wall: Option<Entity> = side_walls_q
                .iter()
                .find(|(_, sw)| sw.0 == entity)
                .map(|(e, _)| e);

            match node.depth {
                None => {
                    // No depth, iff this was extruded before despawn that child entity only.
                    if let Some(wall_entity) = existing_wall {
                        commands.entity(wall_entity).despawn();
                    }
                }
                Some(depth) => {
                    let [r, g_c, b, a] = node.color;
                    let color = Color::srgba(
                        r as f32 / 255.0,
                        g_c as f32 / 255.0,
                        b as f32 / 255.0,
                        a as f32 / 255.0,
                    );
                    let side_mesh = crate::extrude::build_text_side_walls(
                        &atlas, font.0, &node.text, half_adv, depth, 6,
                    );
                    let side_mesh_handle = meshes.add(side_mesh);
                    // TODO: double_sided and cull_mode are not right but I was lazy.
                    let side_mat_handle = std_materials.add(StandardMaterial {
                        base_color: color,
                        alpha_mode: if a < 255 {
                            bevy::render::alpha::AlphaMode::Blend
                        } else {
                            bevy::render::alpha::AlphaMode::Opaque
                        },
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    });

                    if let Some(wall_entity) = existing_wall {
                        // Update the existing child mesh and material in place.
                        commands
                            .entity(wall_entity)
                            .insert((Mesh3d(side_mesh_handle), MeshMaterial3d(side_mat_handle)));
                    } else {
                        // Spawn a new side-wall child that shares the parents
                        // transform.
                        let wall_entity = commands
                            .spawn((
                                Mesh3d(side_mesh_handle),
                                MeshMaterial3d(side_mat_handle),
                                Transform::default(),
                                Visibility::default(),
                                SlugSideWallOf(entity),
                            ))
                            .id();
                        commands.entity(entity).add_child(wall_entity);
                    }
                }
            }
            continue;
        }

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
            let [r, g_c, b, a] = node.color;
            mat.text_color = bevy::math::Vec4::new(
                r as f32 / 255.0,
                g_c as f32 / 255.0,
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
        let mesh = build_mesh_from_run(&run, None);

        if let Ok(existing) = mesh2d_q.get(entity)
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

/// Webgl version of sync_text_meshes, the 3d path uses data textures at group
/// 3.
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
            Option<&MeshMaterial3d<SlugMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugMaterial>>,
        Option<&MeshMaterial3d<SlugMaterial>>,
    )>,
    mesh2d_q: Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: Query<&Mesh3d>,
    side_walls_q: Query<(Entity, &SlugSideWallOf)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SlugMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
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
                Option<&MeshMaterial3d<SlugMaterial>>,
            ),
        >,
    > = if flag.0 {
        Box::new(all_q.iter())
    } else {
        Box::new(changed_q.iter())
    };

    for (entity, node, font, mat_node, mat3d_node) in iter {
        if let Some(mm3d) = mat3d_node {
            use bevy::asset::RenderAssetUsages;
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

            const INTERNAL_SIZE: f32 = 1000.0;
            let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                continue;
            };
            if run.is_empty() {
                continue;
            }

            let inv_h = if run.natural_height > 0.0 {
                1.0 / run.natural_height
            } else {
                1.0
            };

            let normalized_layout: Vec<crate::slug::SlugGlyphLayout> = run
                .glyph_layout
                .iter()
                .map(|g| crate::slug::SlugGlyphLayout {
                    screen_rect: [
                        g.screen_rect[0] * inv_h,
                        g.screen_rect[1] * inv_h,
                        g.screen_rect[2] * inv_h,
                        g.screen_rect[3] * inv_h,
                    ],
                    em_rect: g.em_rect,
                    glyph_index: g.glyph_index,
                    _pad: g._pad,
                })
                .collect();

            if let Some(mat) = materials.get_mut(mm3d) {
                let width = crate::SLUG_TEX_WIDTH;
                let usage = RenderAssetUsages::RENDER_WORLD;

                let [r, g_c, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.params.layout_flags = node.layout.to_u32();

                let runs_data: [f32; 4] = [
                    run.natural_advance * inv_h,
                    1.0,
                    0f32,
                    normalized_layout.len() as f32,
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

                let count = normalized_layout.len();
                let texels = count * 3;
                let height = ((texels as u32).div_ceil(width)).max(1);
                let total_floats = (width * height) as usize * 4;
                let mut px: Vec<f32> = vec![0.0f32; total_floats];
                for (gi, g) in normalized_layout.iter().enumerate() {
                    let base = gi * 3 * 4;
                    px[base..base + 4].copy_from_slice(&g.screen_rect);
                    px[base + 4..base + 8].copy_from_slice(&g.em_rect);
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

            let half_adv = run.natural_advance * inv_h * 0.5;
            let norm_run = normalize_run_3d(&run, inv_h);
            let mesh = build_mesh_from_run(&norm_run, node.depth);

            if let Ok(existing) = mesh3d_q.get(entity) {
                if let Some(m) = meshes.get_mut(&existing.0) {
                    *m = mesh;
                } else {
                    let handle = meshes.add(mesh);
                    commands.entity(entity).insert(Mesh3d(handle));
                }
            } else {
                let handle = meshes.add(mesh);
                commands.entity(entity).insert(Mesh3d(handle));
            }

            // Side-wall child entity management in webgl is similar to native.
            //
            // Can't wait to rip the webgl paths out of everything so bad made
            // all of this 300 times harder than it need be.
            let existing_wall: Option<Entity> = side_walls_q
                .iter()
                .find(|(_, sw)| sw.0 == entity)
                .map(|(e, _)| e);

            match node.depth {
                None => {
                    if let Some(wall_entity) = existing_wall {
                        commands.entity(wall_entity).despawn();
                    }
                }
                Some(depth) => {
                    let [r, g_c, b, a] = node.color;
                    let color = Color::srgba(
                        r as f32 / 255.0,
                        g_c as f32 / 255.0,
                        b as f32 / 255.0,
                        a as f32 / 255.0,
                    );
                    let side_mesh = crate::extrude::build_text_side_walls(
                        &atlas, font.0, &node.text, half_adv, depth, 6,
                    );
                    let side_mesh_handle = meshes.add(side_mesh);
                    let side_mat_handle = std_materials.add(StandardMaterial {
                        base_color: color,
                        alpha_mode: if a < 255 {
                            bevy::render::alpha::AlphaMode::Blend
                        } else {
                            bevy::render::alpha::AlphaMode::Opaque
                        },
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    });

                    if let Some(wall_entity) = existing_wall {
                        commands
                            .entity(wall_entity)
                            .insert((Mesh3d(side_mesh_handle), MeshMaterial3d(side_mat_handle)));
                    } else {
                        let wall_entity = commands
                            .spawn((
                                Mesh3d(side_mesh_handle),
                                MeshMaterial3d(side_mat_handle),
                                Transform::default(),
                                Visibility::default(),
                                SlugSideWallOf(entity),
                            ))
                            .id();
                        commands.entity(entity).add_child(wall_entity);
                    }
                }
            }
            continue;
        }

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
                let [r, g_c, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.params.layout_flags = node.layout.to_u32();

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

                let count = run.glyph_layout.len();
                let texels = count * 3;
                let height = ((texels as u32).div_ceil(width)).max(1);
                let total_floats = (width * height) as usize * 4;
                let mut px: Vec<f32> = vec![0.0f32; total_floats];
                for (gi, g) in run.glyph_layout.iter().enumerate() {
                    let base = gi * 3 * 4;
                    px[base..base + 4].copy_from_slice(&g.screen_rect);
                    px[base + 4..base + 8].copy_from_slice(&g.em_rect);
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

        let mesh = build_mesh_from_run(&run, None);
        if let Ok(existing) = mesh2d_q.get(entity) {
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

/// Keep `SlugMaterial::local_to_clip` in sync with the 3-D entity's transform.
///
/// Runs in PostUpdate after TransformPropagate so every `GlobalTransform` is
/// finished. Queries all entities matching `MeshMaterial3d<SlugMaterial>` and the
/// single `Camera3d` to compute `projection * view⁻¹ * model`.
fn sync_slug_3d_transforms(
    mut materials: ResMut<Assets<SlugMaterial>>,
    text_q: Query<(&GlobalTransform, &MeshMaterial3d<SlugMaterial>)>,
    camera_q: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
) {
    let Ok((cam_gt, proj)) = camera_q.single() else {
        return;
    };
    let clip_from_view = proj.get_clip_from_view();
    let view_from_world = cam_gt.to_matrix().inverse();
    let clip_from_world = clip_from_view * view_from_world;

    for (entity_gt, mat_handle) in text_q.iter() {
        let Some(mat) = materials.get_mut(mat_handle) else {
            continue;
        };
        let world_from_local = entity_gt.to_matrix();
        mat.local_to_clip = (clip_from_world * world_from_local).to_cols_array_2d();
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

/// Return a copy of `run` whose vertices are:
///   - normalized so the full text is exactly 1 world unit tall
///   - Y-flipped: slug uses screen-Y (0 = ascender, 1 = descender), Bevy
///     world-Y increases upward, so we negate and shift to [-0.5, +0.5]
///   - centered at the local origin: X shifted by -half_advance, Y centered
///
/// The coordinate transform applied to each vertex position:
///   new_x =  old_x * inv_h - natural_advance * inv_h * 0.5
///   new_y = -(old_y * inv_h) + 0.5
fn normalize_run_3d(run: &SlugTextRun, inv_h: f32) -> SlugTextRun {
    use crate::slug::SlugVertex;
    let half_adv = run.natural_advance * inv_h * 0.5;
    let vertices: Vec<SlugVertex> = run
        .vertices
        .iter()
        .map(|v| SlugVertex {
            pos: [
                v.pos[0] * inv_h - half_adv,
                -(v.pos[1] * inv_h) + 0.5,
                v.pos[2],
                v.pos[3],
            ],
            glyph: v.glyph,
            color: v.color,
        })
        .collect();
    SlugTextRun {
        vertices,
        indices: run.indices.clone(),
        glyph_layout: run.glyph_layout.clone(),
        font_id: run.font_id,
        natural_advance: run.natural_advance * inv_h,
        natural_height: 1.0,
    }
}

/// Build a `Mesh` from a `SlugTextRun` using the 28-byte slug vertex layout.
///
/// When `depth` is `Some(d)` the function produces both a front face at `+d/2`
/// in local Z coord system and a back face at `-d/2`, with reversed winding so
/// normals point outward. When `depth` is `None` all vertices sit at z=0.0 and
/// it just looks like a 2d decal in 3d space.
///
/// TODO: Here too I need to start making all this wesl/rust bridge data built
/// off of rust sources of truth. I have a lot of comments to remember to update
/// crap all over. And I don't.
/// Attribute layout that matches vertex shader `@location` bindings in mesh3d.wesl:
///   @location(0) "slug_pos"   Vec4  = local-space xy, z=depth offset, w=corner sign
///   @location(1) "slug_glyph" UVec2 = [packed em coords, glyph_index]
///   @location(2) "slug_color" Vec4  = RGBA8 unpacked to Vec4
fn build_mesh_from_run(run: &SlugTextRun, depth: Option<f32>) -> Mesh {
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

    let n = run.vertices.len();
    let capacity = if depth.is_some() { n * 2 } else { n };

    let mut positions: Vec<[f32; 4]> = Vec::with_capacity(capacity);
    let mut glyphs: Vec<[u32; 2]> = Vec::with_capacity(capacity);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(capacity);
    let mut indices: Vec<u32> = Vec::new();

    let color_of = |v: &crate::slug::SlugVertex| {
        let [r, g, b, a] = v.color;
        [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    };

    match depth {
        None => {
            // Flat/decal all vertices at z=0.0, indices are stupid/unchanged.
            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], 0.0, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            indices.extend_from_slice(&run.indices);
        }
        Some(d) => {
            let half = d * 0.5;

            // Front face: z = +half_depth with original winding.
            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], half, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            indices.extend_from_slice(&run.indices);

            // Back face: z = -half_depth with reversed winding aka flip each
            // triangle.
            let back_base = n as u32;
            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], -half, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            // run.indices is just a flat list of triangles as 3 raw indices.
            for tri in run.indices.chunks_exact(3) {
                // Reverse: [i0, i1, i2] -> [i0, i2, i1] for outward back-face normal.
                indices.push(back_base + tri[0]);
                indices.push(back_base + tri[2]);
                indices.push(back_base + tri[1]);
            }
        }
    }

    mesh.insert_attribute(pos_attr, VertexAttributeValues::Float32x4(positions));
    mesh.insert_attribute(glyph_attr, VertexAttributeValues::Uint32x2(glyphs));
    mesh.insert_attribute(color_attr, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));

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
