// Slug text plugin DEAD PLUGIN WALKING
//
// Architecture:
//   SlugAtlas (Resource)     - permanent CPU glyph cache + frame atlas compaction
//   SlugTextNode (Component) - a text string to render (content, font, size, color)
//   SlugTextFont (Component) - which registered FontId to use for an entity
//   Text3dDirty  (Component) - marker: entity needs material upload + mesh build
//   SlugPlugin               - registers materials, inserts resources, adds systems
//
// System order per tick:
//   0. init_slug_entity           - on Added<SlugTextNode>: insert SlugMaterial or StandardMaterial
//   1. collect_and_validate_glyphs  - for all changed SlugTextNodes, calls validate_glyphs
//   2. build_frame_atlas_system   - compact visible glyphs; sets AtlasDirtyFlag
//   3. upload_atlas_system        - if atlas dirty OR any Text3dDirty entity has uninit material,
//                                   upload atlas to all SlugMaterials; sets flag=true on dirty case
//   4. sync_text_meshes           - processes Changed/Added entities + all Text3dDirty entities
//                                   (or all_q when flag=true); removes Text3dDirty on success/reject
//
// The bs above is why the Atlas bs needs to be its own thing. "now there are
// two of them" is basically what I am realizing for all the abuse of slug
// shader bs in all these materials it makes zero sense to duplicate all this
// code. Need to make the atlas a Resource most likely that just gets shared
// between things. Not sure if I share the entire atlas for every shaderbuffer
// or just make everything share the same shader storage buffer? Have to noodle
// on that one a bit.
use bevy::prelude::*;

/// World-space flatness tolerance for adaptive bezier subdivision of extruded
/// side-wall curves.
///
/// Each quadratic bezier gets `ceil(sqrt(flatness / tolerance))` segments so
/// tight curves receive more segments than flat ones automatically. The mesh
/// is normalized to 1 world unit tall, so this is a fraction of letter height:
///
/// ```text
///   0.005  ->  coarse, ~3–5 segments on typical curves
///   0.001  ->  fine,   ~6–12 segments (default, sub-pixel error at 100 px/unit)
///   0.0002 ->  very fine, near-perfect curves at the cost of more geometry
/// ```
const SIDE_WALL_CURVE_TOLERANCE: f32 = 0.001;

/// World-space XY outset applied to every side-wall vertex along its outward
/// edge normal.
///
/// A naive per-edge outset works on convex corners but breaks concave corners
/// (e.g. where the T crossbar meets the stem): those edges' right-perpendicular
/// normals point inward, so the outset pushes verts into the letter, causing
/// adjacent quads to cross and produce the "cross/inset" artifact.
///
/// Proper offsetting requires miter joints angle-bisector scaling at each
/// corner. Until that is implemented, leave this at 0.0 and rely on
/// `SIDE_WALL_CURVE_TOLERANCE` to control linearization quality.
const SIDE_WALL_XY_OUTSET: f32 = 0.0;

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderBuffer;

use crate::{
    SlugAtlasLayout,
    layout::Layout,
    slug::{FontId, SlugAtlas, SlugTextRun},
};

#[allow(unused_imports)]
use crate::slug_text_material::{
    SlugAtlasImages, SlugText3dTextureMaterial, SlugTextMaterialPlugin, SlugTextTextureMaterial,
};

#[cfg(not(feature = "webgl"))]
use crate::slug_text_material::{SlugAtlasBuffers, SlugText3dMaterial, SlugTextMaterial};

/// Text content and rendering parameters for a slug text entity.
///
/// Positioning and alignment are handled entirely in the shader via `slugtext()`.
/// `SlugTextNode` carries only the intrinsic properties of the text itself
/// what to render, at what size, and in what color. The where and how are
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
    /// `-depth/2` along the local z axis or y in bevy coord system, and spawns a
    /// side-wall child entity for both slug "caps".
    pub depth: Option<f32>,
}

impl Default for SlugTextNode {
    fn default() -> Self {
        SlugTextNode {
            text: String::new(),
            font_size: 24.0,
            color: [255, 255, 255, 255],
            layout: Layout::default(),
            depth: None,
        }
    }
}

/// Which font via [`FontId`]) this entity renders with.
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
/// Spawn this bundle and the [`SlugPlugin`] systems will automatically insert
/// the correct material (`MeshMaterial3d<StandardMaterial>` for extruded text,
/// `MeshMaterial3d<SlugMaterial>` for flat text) and populate `mesh` on the
/// first and every subsequent changed frame. Callers never need to create
/// materials themselves. For now. For later it would be neat to allow a caller
/// to change say emissivity to make this crap glow or whatever.
///
/// # Example
/// ```no_run
/// commands.spawn(SlugTextMesh {
///     node: SlugTextNode { text: "Hello".into(), color: [255,255,255,255], ..default() },
///     font: SlugTextFont(my_font_id),
///     transform: Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::splat(2.0)),
///     ..default()
/// });
/// ```
#[derive(Bundle, Default)]
pub struct SlugTextMesh {
    pub node: SlugTextNode,
    pub font: SlugTextFont,
    pub mesh: Mesh3d,
    pub transform: Transform,
    pub visibility: Visibility,
}

/// Marker resource present on webgl until the atlas textures have been
/// uploaded for the first time. Avoids non initialized frame update issues in webgl.
#[derive(Resource)]
pub struct SlugAtlasNotReady;

/// Marker component placed on a [`SlugTextNode`] entity that still needs its
/// material and mesh fully initialized or reuploaded.
#[derive(Component)]
pub struct Text3dDirty;

/// Runs on every newly-spawned `SlugTextNode` entity and inserts the correct
/// rendering components so callers never have to create materials themselves.
///
/// - `depth = Some(_)` -> extruded 3D: `Mesh3d::default()` + `MeshMaterial3d<StandardMaterial>`
/// - `depth = None`, no `Node` -> flat world-space: `Mesh3d::default()` + `MeshMaterial3d<SlugText3dMaterial>` (native)
///   or `MeshMaterial3d<SlugText3dTextureMaterial>` (webgl)
/// - `depth = None`, has `Node` -> UI: skip ui entities are set up differently
///
/// The empty `Mesh3d` handle is fine `sync_text_meshes` populates it on the
/// same or next frame via `Added<SlugTextFont>`.
#[allow(clippy::type_complexity)]
pub fn init_slug_entity(
    #[cfg(not(feature = "webgl"))] query: Query<
        (Entity, &SlugTextNode),
        (
            Added<SlugTextNode>,
            Without<MeshMaterial3d<SlugText3dMaterial>>,
            Without<MeshMaterial3d<StandardMaterial>>,
            Without<MaterialNode<SlugTextMaterial>>,
        ),
    >,
    #[cfg(feature = "webgl")] query: Query<
        (Entity, &SlugTextNode),
        (
            Added<SlugTextNode>,
            Without<MeshMaterial3d<SlugText3dTextureMaterial>>,
            Without<MeshMaterial3d<StandardMaterial>>,
            Without<MaterialNode<SlugTextTextureMaterial>>,
        ),
    >,
    // TODO: type defs for the material stuff to simplify these beasts and
    // remove cfg gates in a fn definition. I am a heretic for this code.
    #[cfg(not(feature = "webgl"))] mut slug3d_materials: ResMut<Assets<SlugText3dMaterial>>,
    #[cfg(feature = "webgl")] mut slug3d_texture_materials: ResMut<
        Assets<SlugText3dTextureMaterial>,
    >,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) {
    for (entity, node) in &query {
        if node.depth.is_some() {
            let [r, g, b, a] = node.color;
            let color = bevy::color::Color::srgba(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            );
            let mat = std_materials.add(StandardMaterial {
                base_color: color,
                alpha_mode: if a < 255 {
                    bevy::material::AlphaMode::Blend
                } else {
                    bevy::material::AlphaMode::Opaque
                },
                double_sided: true,
                cull_mode: None,
                ..default()
            });
            commands
                .entity(entity)
                .insert((Mesh3d::default(), MeshMaterial3d(mat)));
        } else {
            #[cfg(not(feature = "webgl"))]
            {
                use bevy::asset::RenderAssetUsages;
                use bevy::render::render_resource::BufferUsages;
                let zeros = [0u8; 96];
                let mut b = ShaderBuffer::new(&zeros, RenderAssetUsages::RENDER_WORLD);
                b.buffer_description.usage = BufferUsages::UNIFORM | BufferUsages::COPY_DST;
                let params_handle = buffers.add(b);
                let mat = slug3d_materials.add(SlugText3dMaterial {
                    params_buf: Some(params_handle),
                    ..Default::default()
                });
                commands
                    .entity(entity)
                    .insert((Mesh3d::default(), MeshMaterial3d(mat)));
            }
            #[cfg(feature = "webgl")]
            {
                let mat = slug3d_texture_materials.add(SlugText3dTextureMaterial::default());
                commands
                    .entity(entity)
                    .insert((Mesh3d::default(), MeshMaterial3d(mat)));
            }
        }
    }
}

// TODO: This is a crap api, need to improve it. This is "make it work" level quality.
/// Bevy plugin that registers all the slug text infrastructure.
///
/// After adding this plugin:
/// 1. Call `world.resource_mut::<SlugAtlas>().register_font(bytes)` to register
///    a font and get a `FontId`.
/// 2. Spawn an entity with [`SlugTextNode`] + [`SlugTextFont`] + a
///    `MaterialMeshBundle<SlugMaterial>` or `MaterialNode<SlugMaterial>`.
pub struct SlugPlugin;

impl Plugin for SlugPlugin {
    fn build(&self, app: &mut App) {
        // Register all four new material pipelines and insert SlugAtlasBuffers and/or
        // SlugAtlasImages resources. SlugTextMaterialPlugin must be added after
        // ShadersPlugin (which registers the shader handles it references).
        app.add_plugins(SlugTextMaterialPlugin);

        // On webgl, block sync_text_meshes from running until the atlas is ready.
        // The marker is removed by upload_atlas_system on the first successful
        // upload.
        #[cfg(feature = "webgl")]
        app.insert_resource(SlugAtlasNotReady);

        // Insert the combined atlas resource.
        app.insert_resource(SlugAtlas::default());

        // Tracks whether the frame atlas changed this cycle.
        app.insert_resource(AtlasDirtyFlag(false));

        // Extra glyph IDs contributed by non-SlugTextNode sources TypstTextNode mvp only atm.
        // Drained each frame by build_frame_atlas_system.
        app.init_resource::<ExtraGlyphNeeds>();

        // Mirrors the StatsOverlay write_buffer pattern where main-world systems
        // write into SlugParamsUploadMap instead of calling materials.get_mut,
        // so the material asset never gets dirty and the bind group is never
        // recreated. The ExtractResourcePlugin copies the map to the render
        // world each frame; upload_slug_params flushes it via write_buffer.
        #[cfg(not(feature = "webgl"))]
        {
            use bevy::render::extract_resource::ExtractResourcePlugin;
            use bevy::render::{Render, RenderApp, RenderSystems};
            app.add_plugins(ExtractResourcePlugin::<crate::SlugParamsUploadMap>::default())
                .init_resource::<crate::SlugParamsUploadMap>()
                .add_plugins(ExtractResourcePlugin::<crate::SlugAtlasUploadMap>::default())
                .init_resource::<crate::SlugAtlasUploadMap>()
                .add_plugins(ExtractResourcePlugin::<crate::SlugDrawUploadMap>::default())
                .init_resource::<crate::SlugDrawUploadMap>();

            if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
                render_app.add_systems(
                    Render,
                    (
                        crate::upload_slug_params,
                        crate::upload_slug_atlas,
                        crate::upload_slug_draw,
                    )
                        .in_set(RenderSystems::Queue),
                );
            }
        }

        // init_slug_entity MUST run before the chain: sync_text_meshes queries
        // mat3d_slug or mat_node which only exist after init_slug_entity inserts
        // them. If init_slug_entity hasn't run yet, mat3d_slug is None for a
        // newly-spawned entity and sync_text_meshes falls through to the wrong
        // path inserting Mesh2d instead of Mesh3d, leaving the draw_buffer empty.
        app.configure_sets(Update, SlugAtlasSet);
        app.add_systems(Update, init_slug_entity.before(SlugAtlasSet));
        app.add_systems(
            Update,
            (
                collect_and_validate_glyphs,
                build_frame_atlas_system,
                upload_atlas_system,
                sync_text_meshes.run_if(not(resource_exists::<SlugAtlasNotReady>)),
                sync_text_meshes_texture.run_if(not(resource_exists::<SlugAtlasNotReady>)),
            )
                .chain()
                .in_set(SlugAtlasSet),
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

/// Set to true when build_frame_atlas returns true when cleared after sync_text_meshes.
#[derive(Resource, Default)]
pub(crate) struct AtlasDirtyFlag(bool);

/// Public [`SystemSet`] that spans the entire slug atlas build + upload + mesh
/// sync chain in [`Update`].
///
/// Use this for ordering external systems that depend on the frame atlas:
/// ```rust
/// // runs before the atlas is rebuilt safe to write ExtraGlyphNeeds
/// app.add_systems(Update, my_prepare.before(SlugAtlasSet));
/// // runs after atlas and SSBs are uploaded safe to read atlas indices
/// app.add_systems(Update, my_build.after(SlugAtlasSet));
/// ```
///
/// TODO: All this Atlas crap needs to be its own struct/impl for Flan needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct SlugAtlasSet;

/// Extra glyph IDs to include in the next `build_frame_atlas_system` run,
/// contributed by non-[`SlugTextNode`] sources such as [`crate::typst_text`].
///
/// Writers add entries before `build_frame_atlas_system` runs the system
/// drains this resource each frame so stale entries do not accumulate.
#[derive(Resource, Default)]
pub struct ExtraGlyphNeeds(pub Vec<(crate::slug::FontId, Vec<u16>)>);

/// For every changed SlugTextNode, call validate_glyphs so the cpu cache is warm.
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

/// Compact the visible glyph set into gpu buffers. Runs after validate_glyphs.
///
/// Also drains [`ExtraGlyphNeeds`] so that non-[`SlugTextNode`] sources
/// (e.g. `TypstTextNode`) have their shaped glyph IDs included in the frame
/// atlas and accessible via `atlas.frame.glyph_index` in the same frame.
fn build_frame_atlas_system(
    mut atlas: ResMut<SlugAtlas>,
    all_q: Query<(&SlugTextNode, &SlugTextFont)>,
    mut flag: ResMut<AtlasDirtyFlag>,
    mut extra: ResMut<ExtraGlyphNeeds>,
) {
    let mut per_font: std::collections::HashMap<FontId, Vec<u16>> =
        std::collections::HashMap::new();

    for (node, font) in all_q.iter() {
        let ids = atlas.collect_glyph_ids(font.0, &node.text);
        per_font.entry(font.0).or_default().extend(ids);
    }

    // Drain extra glyph needs from TypstTextNode for now.
    for (font_id, glyph_ids) in extra.0.drain(..) {
        per_font.entry(font_id).or_default().extend(glyph_ids);
    }

    let mut needed: Vec<(FontId, Vec<u16>)> = per_font.into_iter().collect();
    needed.sort_unstable_by_key(|(fid, _)| fid.0);
    let rebuilt = atlas.build_frame_atlas(&needed);
    flag.0 = rebuilt;
}

/// Allocate or reuse-via-write_buffer a single atlas SSB section.
///
/// If the existing buffer has enough capacity, schedules a `write_buffer` via
/// `atlas_upload` so the render world uploads the bytes without touching the
/// bind group. Otherwise allocates a new SSB with 50% headroom and updates
/// `current_handle` and `current_cap` in place.
#[cfg(not(feature = "webgl"))]
fn upload_atlas_section(
    data: &[u8],
    current_handle: &mut Handle<ShaderBuffer>,
    current_cap: &mut u64,
    atlas_upload: &mut crate::SlugAtlasUploadMap,
    buffers: &mut Assets<ShaderBuffer>,
) {
    use bevy::asset::{AssetId, RenderAssetUsages};
    use bevy::render::render_resource::BufferUsages;

    let sz = data.len() as u64;
    if *current_cap >= sz && current_handle.id() != AssetId::default() {
        atlas_upload
            .entries
            .insert(current_handle.id(), data.to_vec());
        return;
    }
    let cap = (data.len() * 3 / 2).max(data.len());
    let mut alloc = vec![0u8; cap];
    alloc[..data.len()].copy_from_slice(data);
    let mut ssb = ShaderBuffer::new(
        &alloc,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    ssb.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
    if current_handle.id() != AssetId::default()
        && let Some(mut buf) = buffers.get_mut(current_handle.id())
    {
        *buf = ssb;
        *current_cap = cap as u64;
    } else {
        let h = buffers.add(ssb);
        *current_handle = h;
        *current_cap = cap as u64;
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(not(feature = "webgl"))]
fn upload_atlas_system(
    mut flag: ResMut<AtlasDirtyFlag>,
    atlas: Res<SlugAtlas>,
    dirty_mat3d_q: Query<&MeshMaterial3d<SlugText3dMaterial>, With<Text3dDirty>>,
    all_mat3d_q: Query<&MeshMaterial3d<SlugText3dMaterial>>,
    all_mat_node_q: Query<&MaterialNode<SlugTextMaterial>>,
    mut mat3d_assets: ResMut<Assets<SlugText3dMaterial>>,
    mut mat_ui_assets: ResMut<Assets<SlugTextMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut atlas_buffers: ResMut<SlugAtlasBuffers>,
    mut atlas_upload: ResMut<crate::SlugAtlasUploadMap>,
) {
    // Check if any dirty entity has an uninitialized atlas (curves == default handle).
    let dirty_needs_upload = dirty_mat3d_q.iter().any(|mm| {
        mat3d_assets
            .get(&mm.0)
            .is_some_and(|m| m.curves.id() == bevy::asset::AssetId::default())
    });

    if !flag.0 && !dirty_needs_upload {
        return;
    }

    atlas_upload.entries.clear();

    let layout = SlugAtlasLayout {
        curves_data: atlas.frame.curves.clone(),
        curve_indices_data: atlas.frame.curve_indices.clone(),
        glyphs_data: atlas.frame.glyphs.clone(),
    };

    let c_sz = layout.curves_data.len();
    let ci_sz = layout.curve_indices_data.len();
    let g_sz = layout.glyphs_data.len();

    if c_sz == 0 || ci_sz == 0 || g_sz == 0 {
        if dirty_needs_upload {
            flag.0 = true;
        }
        return;
    }

    // Reborrow as a plain &mut reference so Rust can split-borrow the fields.
    let ab = &mut *atlas_buffers;
    upload_atlas_section(
        &layout.curves_data,
        &mut ab.curves,
        &mut ab.curves_cap,
        &mut atlas_upload,
        &mut buffers,
    );
    upload_atlas_section(
        &layout.curve_indices_data,
        &mut ab.curve_indices,
        &mut ab.ci_cap,
        &mut atlas_upload,
        &mut buffers,
    );
    upload_atlas_section(
        &layout.glyphs_data,
        &mut ab.glyphs,
        &mut ab.glyphs_cap,
        &mut atlas_upload,
        &mut buffers,
    );

    // Point every SlugText3dMaterial instance at the shared atlas handles.
    for mm in all_mat3d_q.iter() {
        if let Some(mut mat) = mat3d_assets.get_mut(&mm.0) {
            mat.curves = atlas_buffers.curves.clone();
            mat.curve_indices = atlas_buffers.curve_indices.clone();
            mat.glyphs = atlas_buffers.glyphs.clone();
        }
    }

    // Point every SlugTextMaterial UIMaterial instance at the shared atlas handles.
    for mn in all_mat_node_q.iter() {
        if let Some(mut mat) = mat_ui_assets.get_mut(&mn.0) {
            mat.curves = atlas_buffers.curves.clone();
            mat.curve_indices = atlas_buffers.curve_indices.clone();
            mat.glyphs = atlas_buffers.glyphs.clone();
        }
    }

    if dirty_needs_upload {
        flag.0 = true;
    }
}

/// Upload atlas textures into the shared [`SlugAtlasImages`] resource on WebGL,
/// then propagate the handles to all [`SlugText3dTextureMaterial`] and
/// [`SlugTextTextureMaterial`] instances.
#[cfg(feature = "webgl")]
fn upload_atlas_system(
    mut flag: ResMut<AtlasDirtyFlag>,
    atlas: Res<SlugAtlas>,
    dirty_mat3d_q: Query<&MeshMaterial3d<SlugText3dTextureMaterial>, With<Text3dDirty>>,
    all_mat3d_q: Query<&MeshMaterial3d<SlugText3dTextureMaterial>>,
    all_mat_node_q: Query<&MaterialNode<SlugTextTextureMaterial>>,
    mut mat3d_assets: ResMut<Assets<SlugText3dTextureMaterial>>,
    mut mat_ui_assets: ResMut<Assets<SlugTextTextureMaterial>>,
    mut atlas_images: ResMut<SlugAtlasImages>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    use bevy::asset::AssetId;

    let dirty_needs_upload = dirty_mat3d_q.iter().any(|mm| {
        mat3d_assets
            .get(&mm.0)
            .is_some_and(|m| m.curves_image.id() == AssetId::default())
    });

    if !flag.0 && !dirty_needs_upload {
        return;
    }

    let layout = SlugAtlasLayout {
        curves_data: atlas.frame.curves.clone(),
        curve_indices_data: atlas.frame.curve_indices.clone(),
        glyphs_data: atlas.frame.glyphs.clone(),
    };

    layout.assert_fits_webgl_textures();

    // Rebuild the shared atlas images and store handles in SlugAtlasImages.
    update_or_add_image(&mut atlas_images.curves, layout.curves_image(), &mut images);
    update_or_add_image(
        &mut atlas_images.curve_indices,
        layout.curve_indices_image(),
        &mut images,
    );
    update_or_add_image(&mut atlas_images.glyphs, layout.glyphs_image(), &mut images);

    // Propagate to all SlugText3dTextureMaterial instances.
    for mm in all_mat3d_q.iter() {
        if let Some(mut mat) = mat3d_assets.get_mut(&mm.0) {
            mat.curves_image = atlas_images.curves.clone();
            mat.curve_indices_image = atlas_images.curve_indices.clone();
            mat.glyphs_image = atlas_images.glyphs.clone();
        }
    }

    // Propagate to all SlugTextTextureMaterial UIMaterial instances.
    for mn in all_mat_node_q.iter() {
        if let Some(mut mat) = mat_ui_assets.get_mut(&mn.0) {
            mat.curves_image = atlas_images.curves.clone();
            mat.curve_indices_image = atlas_images.curve_indices.clone();
            mat.glyphs_image = atlas_images.glyphs.clone();
        }
    }

    flag.0 = true;
    commands.remove_resource::<SlugAtlasNotReady>();
}

/// Pack `SlugRunDesc` + `glyph_layout` into `mat.runs` / `mat.glyph_layout` SSBs
/// for a [`SlugTextMaterial`] (UI, native path).
///
/// When data fits within existing capacity, adds to `draw_upload` for render-world
/// write_buffer. On overflow or first allocation, creates a new SSB with 50% headroom.
#[cfg(not(feature = "webgl"))]
fn pack_draw_buffer_text(
    mat: &mut SlugTextMaterial,
    run_desc: &crate::SlugRunDesc,
    layout: &[crate::slug::SlugGlyphLayout],
    buffers: &mut Assets<ShaderBuffer>,
    draw_upload: &mut crate::SlugDrawUploadMap,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::BufferUsages;

    let runs_bytes: &[u8] = bytemuck::bytes_of(run_desc);
    let layout_bytes: &[u8] = bytemuck::cast_slice(layout);

    if runs_bytes.is_empty() || layout_bytes.is_empty() {
        return;
    }

    if mat.runs.id() != bevy::asset::AssetId::default() {
        draw_upload
            .entries
            .insert(mat.runs.id(), runs_bytes.to_vec());
    } else {
        let cap = (runs_bytes.len() * 3 / 2).max(runs_bytes.len());
        let mut alloc = vec![0u8; cap];
        alloc[..runs_bytes.len()].copy_from_slice(runs_bytes);
        let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        let mut ssb = ShaderBuffer::new(&alloc, usage);
        ssb.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
        mat.runs = buffers.add(ssb);
    }

    if mat.glyph_layout.id() != bevy::asset::AssetId::default() {
        draw_upload
            .entries
            .insert(mat.glyph_layout.id(), layout_bytes.to_vec());
    } else {
        let cap = (layout_bytes.len() * 3 / 2).max(layout_bytes.len());
        let mut alloc = vec![0u8; cap];
        alloc[..layout_bytes.len()].copy_from_slice(layout_bytes);
        let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        let mut ssb = ShaderBuffer::new(&alloc, usage);
        ssb.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
        mat.glyph_layout = buffers.add(ssb);
    }
}

/// Pack run+glyph_layout images into a [`SlugTextTextureMaterial`] UIMaterial texture path.
#[cfg(feature = "webgl")]
fn pack_draw_images_texture(
    mat: &mut SlugTextTextureMaterial,
    run_desc: &crate::SlugRunDesc,
    layout: &[crate::slug::SlugGlyphLayout],
    images: &mut Assets<Image>,
) {
    let new_runs = crate::build_runs_image(run_desc);
    let new_layout = crate::build_glyph_layout_image(layout);
    update_or_add_image(&mut mat.runs_image, new_runs, images);
    update_or_add_image(&mut mat.glyph_layout_image, new_layout, images);
}

/// Inner loop shared by both `sync_text_meshes` variants.
///
/// Entities are processed from `iter`. For each entity:
/// - If the material isn't on the entity yet (deferred Command not flushed),
///   skip it. The [`Text3dDirty`] marker will keep it in scope next frame.
/// - On successful mesh/upload, remove [`Text3dDirty`] from the entity.
/// - On a rejected change aka backing font is missing glyphs, remove [`Text3dDirty`].
///
/// The `mat3d_slug` slot carries:
///   native -> `Option<&MeshMaterial3d<SlugText3dMaterial>>`
///   webgl  -> `Option<&MeshMaterial3d<SlugText3dTextureMaterial>>`
///
/// The `mat_node` slot carries:
///   native -> `Option<&MaterialNode<SlugTextMaterial>>`
///   webgl  -> `Option<&MaterialNode<SlugTextTextureMaterial>>`
#[allow(clippy::too_many_arguments)]
#[cfg(not(feature = "webgl"))]
fn sync_text_meshes_inner<'w>(
    atlas: &SlugAtlas,
    flag: &mut AtlasDirtyFlag,
    iter: impl Iterator<
        Item = (
            Entity,
            &'w SlugTextNode,
            &'w SlugTextFont,
            Option<&'w MaterialNode<SlugTextMaterial>>,
            Option<&'w MeshMaterial3d<SlugText3dMaterial>>,
            Option<&'w MeshMaterial3d<StandardMaterial>>,
        ),
    >,
    mesh2d_q: &Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: &Query<&Mesh3d>,
    meshes: &mut ResMut<Assets<Mesh>>,
    mat3d_assets: &mut ResMut<Assets<SlugText3dMaterial>>,
    mat_ui_assets: &mut ResMut<Assets<SlugTextMaterial>>,
    std_materials: &mut ResMut<Assets<StandardMaterial>>,
    buffers: &mut ResMut<Assets<ShaderBuffer>>,
    upload_map: &mut crate::SlugParamsUploadMap,
    draw_upload: &mut crate::SlugDrawUploadMap,
    commands: &mut Commands,
) {
    for (entity, node, font, mat_node, mat3d_slug, mat3d_std) in iter {
        if mat_node.is_none() && mat3d_slug.is_none() && mat3d_std.is_none() {
            continue;
        }

        if node.depth.is_some() {
            if let Some(depth) = node.depth {
                const INTERNAL_SIZE: f32 = 1000.0;
                let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                    bevy::log::warn!("SlugAtlas::shape (extruded) failed for {:?}", entity);
                    commands.entity(entity).remove::<Text3dDirty>();
                    continue;
                };
                if run.is_empty() {
                    commands.entity(entity).remove::<Text3dDirty>();
                    continue;
                }

                let inv_h = if run.natural_height > 0.0 {
                    1.0 / run.natural_height
                } else {
                    1.0
                };
                let half_adv = run.natural_advance * inv_h * 0.5;

                let mesh_3d = crate::extrude::build_text_3d_mesh(
                    atlas,
                    font.0,
                    &node.text,
                    half_adv,
                    depth,
                    SIDE_WALL_CURVE_TOLERANCE,
                    SIDE_WALL_XY_OUTSET,
                );

                if let Ok(existing) = mesh3d_q.get(entity)
                    && let Some(mut m) = meshes.get_mut(&existing.0)
                {
                    *m = mesh_3d;
                } else {
                    commands.entity(entity).insert(Mesh3d(meshes.add(mesh_3d)));
                }

                let [r, g_c, b, a] = node.color;
                let color = Color::srgba(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                if let Some(std_handle) = mat3d_std
                    && let Some(mut mat) = std_materials.get_mut(std_handle)
                {
                    mat.base_color = color;
                    mat.alpha_mode = if a < 255 {
                        bevy::material::AlphaMode::Blend
                    } else {
                        bevy::material::AlphaMode::Opaque
                    };
                }
            }
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        if let Some(mm3d) = mat3d_slug {
            const INTERNAL_SIZE: f32 = 1000.0;
            let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                bevy::log::warn!("SlugAtlas::shape (flat 3d) failed for {:?}", entity);
                commands.entity(entity).remove::<Text3dDirty>();
                continue;
            };
            if run.is_empty() {
                commands.entity(entity).remove::<Text3dDirty>();
                continue;
            }

            let inv_h = if run.natural_height > 0.0 {
                1.0 / run.natural_height
            } else {
                1.0
            };

            if let Some(mut mat) = mat3d_assets.get_mut(mm3d) {
                let [r, g_c, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.is_extruded = false;
                mat.params.layout_flags = node.layout.to_u32();
                mat.params.alpha_discard = 0.01;

                // Pack params -> upload_map (write_buffer in render world).
                if let Some(ref params_handle) = mat.params_buf {
                    let mut bytes = [0u8; 96];
                    bytes[..16].copy_from_slice(bytemuck::bytes_of(&mat.params));
                    bytes[16..32].copy_from_slice(bytemuck::bytes_of(&mat.text_color));
                    bytes[32..96].copy_from_slice(bytemuck::cast_slice(&mat.local_to_clip));
                    upload_map.entries.insert(params_handle.id(), bytes);
                }
            }

            // Build the normalized mesh that carries all glyph vertex data.
            let norm_run = normalize_run_3d(&run, inv_h);
            let mesh = build_mesh_from_run(&norm_run, None);

            if let Ok(existing) = mesh3d_q.get(entity)
                && let Some(mut m) = meshes.get_mut(&existing.0)
            {
                *m = mesh;
            } else {
                commands.entity(entity).insert(Mesh3d(meshes.add(mesh)));
            }
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        let Some(run) = atlas.shape(font.0, &node.text, node.font_size, node.color) else {
            bevy::log::warn!("SlugAtlas::shape (ui) failed for {:?}", entity);
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        };
        if run.is_empty() {
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        if let Some(mn) = mat_node
            && let Some(mut mat) = mat_ui_assets.get_mut(mn)
        {
            let [r, g_c, b, a] = node.color;
            mat.text_color = bevy::math::Vec4::new(
                r as f32 / 255.0,
                g_c as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            );
            mat.params.layout_flags = node.layout.to_u32();

            let run_desc = crate::SlugRunDesc {
                natural_advance: run.natural_advance,
                natural_height: run.natural_height,
                glyph_offset: 0,
                glyph_count: run.glyph_layout.len() as u32,
            };
            pack_draw_buffer_text(&mut mat, &run_desc, &run.glyph_layout, buffers, draw_upload);
            // SlugTextMaterial packs the 96-byte uniform fresh in as_bind_group each frame,
            // so no params_buf write_buffer is needed here. sync_node_size keeps node_size
            // in sync; text_color and layout_flags were already set above.

            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        // Fallback: no material yet - build a 2D mesh placeholder.
        let mesh = build_mesh_from_run(&run, None);
        if let Ok(existing) = mesh2d_q.get(entity)
            && let Some(mut m) = meshes.get_mut(&existing.0)
        {
            *m = mesh;
        } else {
            commands
                .entity(entity)
                .insert(bevy::prelude::Mesh2d(meshes.add(mesh)));
        }
        commands.entity(entity).remove::<Text3dDirty>();
    }

    flag.0 = false;
}

#[allow(clippy::too_many_arguments)]
#[cfg(feature = "webgl")]
fn sync_text_meshes_inner<'w>(
    atlas: &SlugAtlas,
    flag: &mut AtlasDirtyFlag,
    iter: impl Iterator<
        Item = (
            Entity,
            &'w SlugTextNode,
            &'w SlugTextFont,
            Option<&'w MaterialNode<SlugTextTextureMaterial>>,
            Option<&'w MeshMaterial3d<SlugText3dTextureMaterial>>,
            Option<&'w MeshMaterial3d<StandardMaterial>>,
        ),
    >,
    mesh2d_q: &Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: &Query<&Mesh3d>,
    meshes: &mut ResMut<Assets<Mesh>>,
    mat3d_assets: &mut ResMut<Assets<SlugText3dTextureMaterial>>,
    mat_ui_assets: &mut ResMut<Assets<SlugTextTextureMaterial>>,
    std_materials: &mut ResMut<Assets<StandardMaterial>>,
    images: &mut ResMut<Assets<Image>>,
    commands: &mut Commands,
) {
    for (entity, node, font, mat_node, mat3d_slug, mat3d_std) in iter {
        if mat_node.is_none() && mat3d_slug.is_none() && mat3d_std.is_none() {
            continue;
        }

        if node.depth.is_some() {
            if let Some(depth) = node.depth {
                const INTERNAL_SIZE: f32 = 1000.0;
                let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                    bevy::log::warn!("SlugAtlas::shape extruded failed for {:?}", entity);
                    commands.entity(entity).remove::<Text3dDirty>();
                    continue;
                };
                if run.is_empty() {
                    commands.entity(entity).remove::<Text3dDirty>();
                    continue;
                }

                let inv_h = if run.natural_height > 0.0 {
                    1.0 / run.natural_height
                } else {
                    1.0
                };
                let half_adv = run.natural_advance * inv_h * 0.5;

                let mesh_3d = crate::extrude::build_text_3d_mesh(
                    atlas,
                    font.0,
                    &node.text,
                    half_adv,
                    depth,
                    SIDE_WALL_CURVE_TOLERANCE,
                    SIDE_WALL_XY_OUTSET,
                );

                if let Ok(existing) = mesh3d_q.get(entity)
                    && let Some(mut m) = meshes.get_mut(&existing.0)
                {
                    *m = mesh_3d;
                } else {
                    commands.entity(entity).insert(Mesh3d(meshes.add(mesh_3d)));
                }

                let [r, g_c, b, a] = node.color;
                let color = Color::srgba(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                if let Some(std_handle) = mat3d_std
                    && let Some(mut mat) = std_materials.get_mut(std_handle)
                {
                    mat.base_color = color;
                    mat.alpha_mode = if a < 255 {
                        bevy::material::AlphaMode::Blend
                    } else {
                        bevy::material::AlphaMode::Opaque
                    };
                }
            }
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        if let Some(mm3d) = mat3d_slug {
            const INTERNAL_SIZE: f32 = 1000.0;
            let Some(run) = atlas.shape(font.0, &node.text, INTERNAL_SIZE, node.color) else {
                bevy::log::warn!("SlugAtlas::shape (flat 3d) failed for {:?}", entity);
                commands.entity(entity).remove::<Text3dDirty>();
                continue;
            };
            if run.is_empty() {
                commands.entity(entity).remove::<Text3dDirty>();
                continue;
            }

            let inv_h = if run.natural_height > 0.0 {
                1.0 / run.natural_height
            } else {
                1.0
            };

            if let Some(mut mat) = mat3d_assets.get_mut(mm3d) {
                let [r, g_c, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g_c as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.is_extruded = false;
                mat.params.layout_flags = node.layout.to_u32();
                mat.params.alpha_discard = 0.01;
                // local_to_clip kept in sync by sync_slug_3d_transforms.
            }

            let norm_run = normalize_run_3d(&run, inv_h);
            let mesh = build_mesh_from_run(&norm_run, None);

            if let Ok(existing) = mesh3d_q.get(entity)
                && let Some(mut m) = meshes.get_mut(&existing.0)
            {
                *m = mesh;
            } else {
                commands.entity(entity).insert(Mesh3d(meshes.add(mesh)));
            }
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        let Some(run) = atlas.shape(font.0, &node.text, node.font_size, node.color) else {
            bevy::log::warn!("SlugAtlas::shape (ui) failed for {:?}", entity);
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        };
        if run.is_empty() {
            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        if let Some(mn) = mat_node
            && let Some(mut mat) = mat_ui_assets.get_mut(mn)
        {
            let [r, g_c, b, a] = node.color;
            mat.text_color = bevy::math::Vec4::new(
                r as f32 / 255.0,
                g_c as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            );
            mat.params.layout_flags = node.layout.to_u32();

            let run_desc = crate::SlugRunDesc {
                natural_advance: run.natural_advance,
                natural_height: run.natural_height,
                glyph_offset: 0,
                glyph_count: run.glyph_layout.len() as u32,
            };
            pack_draw_images_texture(&mut *mat, &run_desc, &run.glyph_layout, images);

            commands.entity(entity).remove::<Text3dDirty>();
            continue;
        }

        let mesh = build_mesh_from_run(&run, None);
        if let Ok(existing) = mesh2d_q.get(entity)
            && let Some(mut m) = meshes.get_mut(&existing.0)
        {
            *m = mesh;
        } else {
            commands
                .entity(entity)
                .insert(bevy::prelude::Mesh2d(meshes.add(mesh)));
        }
        commands.entity(entity).remove::<Text3dDirty>();
    }

    flag.0 = false;
}

/// Re-shape and upload vertex/index buffers when text or atlas changes (native).
#[cfg(not(feature = "webgl"))]
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn sync_text_meshes(
    atlas: Res<SlugAtlas>,
    mut flag: ResMut<AtlasDirtyFlag>,
    changed_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugTextMaterial>>,
            Option<&MeshMaterial3d<SlugText3dMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    dirty_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugTextMaterial>>,
            Option<&MeshMaterial3d<SlugText3dMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        ),
        With<Text3dDirty>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugTextMaterial>>,
        Option<&MeshMaterial3d<SlugText3dMaterial>>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    mesh2d_q: Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: Query<&Mesh3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mat3d_assets: ResMut<Assets<SlugText3dMaterial>>,
    mut mat_ui_assets: ResMut<Assets<SlugTextMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut upload_map: ResMut<crate::SlugParamsUploadMap>,
    mut draw_upload: ResMut<crate::SlugDrawUploadMap>,
    mut commands: Commands,
) {
    type Row<'w> = (
        Entity,
        &'w SlugTextNode,
        &'w SlugTextFont,
        Option<&'w MaterialNode<SlugTextMaterial>>,
        Option<&'w MeshMaterial3d<SlugText3dMaterial>>,
        Option<&'w MeshMaterial3d<StandardMaterial>>,
    );

    let iter: Box<dyn Iterator<Item = Row<'_>>> = if flag.0 {
        Box::new(all_q.iter())
    } else {
        // Chain changed + dirty, dedup by entity id so an entity that is both
        // Changed and Dirty isn't processed twice in the same frame.
        Box::new(
            changed_q
                .iter()
                .chain(dirty_q.iter().filter(|(e, ..)| !changed_q.contains(*e))),
        )
    };

    upload_map.entries.clear();
    draw_upload.entries.clear();

    sync_text_meshes_inner(
        &atlas,
        &mut flag,
        iter,
        &mesh2d_q,
        &mesh3d_q,
        &mut meshes,
        &mut mat3d_assets,
        &mut mat_ui_assets,
        &mut std_materials,
        &mut buffers,
        &mut upload_map,
        &mut draw_upload,
        &mut commands,
    );
}

#[cfg(feature = "webgl")]
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn sync_text_meshes(
    atlas: Res<SlugAtlas>,
    mut flag: ResMut<AtlasDirtyFlag>,
    changed_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugTextTextureMaterial>>,
            Option<&MeshMaterial3d<SlugText3dTextureMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        ),
        Or<(Changed<SlugTextNode>, Added<SlugTextFont>)>,
    >,
    dirty_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&MaterialNode<SlugTextTextureMaterial>>,
            Option<&MeshMaterial3d<SlugText3dTextureMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        ),
        With<Text3dDirty>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&MaterialNode<SlugTextTextureMaterial>>,
        Option<&MeshMaterial3d<SlugText3dTextureMaterial>>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    mesh2d_q: Query<&bevy::prelude::Mesh2d>,
    mesh3d_q: Query<&Mesh3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mat3d_assets: ResMut<Assets<SlugText3dTextureMaterial>>,
    mut mat_ui_assets: ResMut<Assets<SlugTextTextureMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    type Row<'w> = (
        Entity,
        &'w SlugTextNode,
        &'w SlugTextFont,
        Option<&'w MaterialNode<SlugTextTextureMaterial>>,
        Option<&'w MeshMaterial3d<SlugText3dTextureMaterial>>,
        Option<&'w MeshMaterial3d<StandardMaterial>>,
    );

    let iter: Box<dyn Iterator<Item = Row<'_>>> = if flag.0 {
        Box::new(all_q.iter())
    } else {
        Box::new(
            changed_q
                .iter()
                .chain(dirty_q.iter().filter(|(e, ..)| !changed_q.contains(*e))),
        )
    };

    sync_text_meshes_inner(
        &atlas,
        &mut flag,
        iter,
        &mesh2d_q,
        &mesh3d_q,
        &mut meshes,
        &mut mat3d_assets,
        &mut mat_ui_assets,
        &mut std_materials,
        &mut images,
        &mut commands,
    );
}

/// Keep the per-entity `local_to_clip` uniform in sync with the 3d entities
/// Keep `SlugText3dMaterial::local_to_clip` in sync with the 3d entites
/// world transform.
#[cfg(not(feature = "webgl"))]
fn sync_slug_3d_transforms(
    mut materials: ResMut<Assets<SlugText3dMaterial>>,
    mut upload_map: ResMut<crate::SlugParamsUploadMap>,
    text_q: Query<(&GlobalTransform, &MeshMaterial3d<SlugText3dMaterial>)>,
    camera_q: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
) {
    let Ok((cam_gt, proj)) = camera_q.single() else {
        return;
    };
    let clip_from_view = proj.get_clip_from_view();
    let view_from_world = cam_gt.to_matrix().inverse();
    let clip_from_world = clip_from_view * view_from_world;

    for (entity_gt, mat_handle) in text_q.iter() {
        let Some(mat) = materials.get(mat_handle) else {
            continue;
        };
        let world_from_local = entity_gt.to_matrix();
        let new_local_to_clip = (clip_from_world * world_from_local).to_cols_array_2d();

        if new_local_to_clip == mat.local_to_clip {
            continue;
        }

        let mut bytes = [0u8; 96];
        bytes[..16].copy_from_slice(bytemuck::bytes_of(&mat.params));
        bytes[16..32].copy_from_slice(bytemuck::bytes_of(&mat.text_color));
        bytes[32..96].copy_from_slice(bytemuck::cast_slice(&new_local_to_clip));

        if let Some(ref params_handle) = mat.params_buf {
            upload_map.entries.insert(params_handle.id(), bytes);
        }

        if let Some(mut mat_mut) = materials.get_mut(mat_handle) {
            mat_mut.local_to_clip = new_local_to_clip;
        }
    }
}

/// Keep `SlugText3dTextureMaterial::local_to_clip` in sync.
#[cfg(feature = "webgl")]
fn sync_slug_3d_transforms(
    mut materials: ResMut<Assets<SlugText3dTextureMaterial>>,
    text_q: Query<(&GlobalTransform, &MeshMaterial3d<SlugText3dTextureMaterial>)>,
    camera_q: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
) {
    let Ok((cam_gt, proj)) = camera_q.single() else {
        return;
    };
    let clip_from_view = proj.get_clip_from_view();
    let view_from_world = cam_gt.to_matrix().inverse();
    let clip_from_world = clip_from_view * view_from_world;

    for (entity_gt, mat_handle) in text_q.iter() {
        let Some(mut mat) = materials.get_mut(mat_handle) else {
            continue;
        };
        let world_from_local = entity_gt.to_matrix();
        let new_local_to_clip = (clip_from_world * world_from_local).to_cols_array_2d();
        if new_local_to_clip != mat.local_to_clip {
            mat.local_to_clip = new_local_to_clip;
        }
    }
}

/// Keep `SlugTextMaterial::params.node_size` in sync with the resolved UI node
/// size.
///
/// SlugTextMaterial packs the 96-byte uniform fresh in `as_bind_group`, so
/// there is no `params_buf` write_buffer here just a direct field mutation.
/// The next `as_bind_group` call will pick up the new `node_size`.
#[cfg(not(feature = "webgl"))]
fn sync_node_size(
    q: Query<(&bevy::ui::ComputedNode, &MaterialNode<SlugTextMaterial>)>,
    mut materials: ResMut<Assets<SlugTextMaterial>>,
) {
    for (computed, mat_node) in &q {
        let size = computed.size();
        if size.x <= 0.0 || size.y <= 0.0 {
            continue;
        }
        let needs_update = materials
            .get(mat_node)
            .is_some_and(|m| (m.params.node_size - size).length_squared() > 1e-4);
        if !needs_update {
            continue;
        }
        if let Some(mut mat) = materials.get_mut(mat_node) {
            mat.params.node_size = size;
        }
    }
}

/// Keep `SlugTextTextureMaterial::params.node_size` in sync.
#[cfg(feature = "webgl")]
fn sync_node_size(
    q: Query<(
        &bevy::ui::ComputedNode,
        &MaterialNode<SlugTextTextureMaterial>,
    )>,
    mut materials: ResMut<Assets<SlugTextTextureMaterial>>,
) {
    for (computed, mat_node) in &q {
        let size = computed.size();
        if size.x > 0.0
            && size.y > 0.0
            && let Some(mat) = materials.get(mat_node)
            && (mat.params.node_size - size).length_squared() > 1e-4
        {
            if let Some(mut mat_mut) = materials.get_mut(mat_node) {
                mat_mut.params.node_size = size;
            }
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
pub fn normalize_run_3d(run: &SlugTextRun, inv_h: f32) -> SlugTextRun {
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

/// Build a `Mesh` from a `SlugTextRun` using the slug vertex layout.
///
/// When `depth` is `Some(d)` the function produces both a front face at `+d/2`
/// in local Z coord system and a back face at `-d/2`, with reversed winding so
/// normals point outward. When `depth` is `None` all vertices sit at z=0.0 and
/// it just looks like a 2d decal in 3d space.
///
/// TODO: Here too I need to start making all this wesl/rust bridge data built
/// off of rust sources of truth. I have a lot of comments to remember to update
/// crap all over. And I don't.
///
/// Attribute layout that matches vertex shader `@location` bindings in mesh3d.wesl:
///   @location(0) "slug_pos"   Vec4  = local-space xy, z=depth offset, w=corner sign
///   @location(1) "slug_glyph" UVec2 = [packed em coords, glyph_index]
///   @location(2) "slug_color" Vec4  = RGBA8 unpacked to Vec4
pub fn build_mesh_from_run(run: &SlugTextRun, depth: Option<f32>) -> Mesh {
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
            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], 0.0, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            indices.extend_from_slice(&run.indices);
        }
        Some(d) => {
            let half = d * 0.5;

            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], half, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            indices.extend_from_slice(&run.indices);

            let back_base = n as u32;
            for v in &run.vertices {
                positions.push([v.pos[0], v.pos[1], -half, v.pos[3]]);
                glyphs.push(v.glyph);
                colors.push(color_of(v));
            }
            for tri in run.indices.chunks_exact(3) {
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

fn update_or_add_image(handle: &mut Handle<Image>, new_img: Image, images: &mut Assets<Image>) {
    use bevy::asset::AssetId;
    if handle.id() != AssetId::default()
        && let Some(mut img) = images.get_mut(handle.id())
    {
        *img = new_img;
        return;
    }
    *handle = images.add(new_img);
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_text_meshes_texture(
    atlas: Res<SlugAtlas>,
    mut flag: ResMut<AtlasDirtyFlag>,
    changed_q: Query<
        (
            Entity,
            &SlugTextNode,
            &SlugTextFont,
            Option<&bevy::prelude::MaterialNode<SlugTextTextureMaterial>>,
        ),
        bevy::prelude::Or<(
            bevy::prelude::Changed<SlugTextNode>,
            bevy::prelude::Added<SlugTextFont>,
        )>,
    >,
    all_q: Query<(
        Entity,
        &SlugTextNode,
        &SlugTextFont,
        Option<&bevy::prelude::MaterialNode<SlugTextTextureMaterial>>,
    )>,
    uninit_q: Query<&bevy::prelude::MaterialNode<SlugTextTextureMaterial>>,
    mut mats: bevy::prelude::ResMut<bevy::prelude::Assets<SlugTextTextureMaterial>>,
    mut images: bevy::prelude::ResMut<bevy::prelude::Assets<bevy::prelude::Image>>,
    meshes: bevy::prelude::ResMut<bevy::prelude::Assets<bevy::prelude::Mesh>>,
) {
    use bevy::asset::AssetId;
    use bevy::prelude::Image;

    let has_uninit = uninit_q.iter().any(|mn| {
        mats.get(mn.id())
            .is_some_and(|m| m.runs_image.id() == AssetId::<Image>::default())
    });

    let iter: Box<
        dyn Iterator<
            Item = (
                Entity,
                &SlugTextNode,
                &SlugTextFont,
                Option<&bevy::prelude::MaterialNode<SlugTextTextureMaterial>>,
            ),
        >,
    > = if flag.0 || has_uninit {
        Box::new(all_q.iter())
    } else {
        Box::new(changed_q.iter())
    };

    for (_entity, node, font, mat_node) in iter {
        if node.depth.is_some() {
            continue;
        }

        let Some(run) = atlas.shape(font.0, &node.text, node.font_size, node.color) else {
            continue;
        };
        if run.is_empty() {
            continue;
        }

        if let Some(mn) = mat_node {
            if let Some(mut mat) = mats.get_mut(mn.id()) {
                let [r, g, b, a] = node.color;
                mat.text_color = bevy::math::Vec4::new(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                );
                mat.params.layout_flags = node.layout.to_u32();

                let run_desc = crate::SlugRunDesc {
                    natural_advance: run.natural_advance,
                    natural_height: run.natural_height,
                    glyph_offset: 0,
                    glyph_count: run.glyph_layout.len() as u32,
                };

                let new_runs = crate::build_runs_image(&run_desc);
                let new_layout = crate::build_glyph_layout_image(&run.glyph_layout);

                update_or_add_image(&mut mat.runs_image, new_runs, &mut images);
                update_or_add_image(&mut mat.glyph_layout_image, new_layout, &mut images);
            }
            continue;
        }

        // TODO: not sure I need this anymore.
        let _ = meshes;
    }

    flag.0 = false;
}

/// Minimal Bevy World + atlas bootstrapped with `font` + `text`.
///
/// Returns `(world, entity, font_id)` where the entity has `SlugTextNode`,
/// `SlugTextFont`, and `MaterialNode<SlugTextTextureMaterial>` already attached.
/// Uses `SlugTextTextureMaterial` (always compiled) so tests run without the
/// `webgl` feature gate.
#[cfg(test)]
fn make_test_world_with_ui_entity(
    font_bytes: &[u8],
    text: &str,
) -> (bevy::ecs::world::World, bevy::ecs::entity::Entity, FontId) {
    use bevy::prelude::*;

    let mut world = bevy::ecs::world::World::default();

    world.init_resource::<Assets<SlugTextTextureMaterial>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<AtlasDirtyFlag>();
    world.init_resource::<SlugAtlas>();

    let fid = {
        let mut atlas = world.resource_mut::<SlugAtlas>();
        let fid = atlas
            .register_font(font_bytes.to_vec())
            .expect("font registration");
        atlas.validate_glyphs(fid, text);
        let ids = atlas.collect_glyph_ids(fid, text);
        atlas.build_frame_atlas(&[(fid, ids)]);
        fid
    };

    // atlas is ready so reset dirty marker
    world.resource_mut::<AtlasDirtyFlag>().0 = false;

    let mat_handle = {
        let mut mats = world.resource_mut::<Assets<SlugTextTextureMaterial>>();
        mats.add(SlugTextTextureMaterial::default())
    };

    let entity = world
        .spawn((
            SlugTextNode {
                text: text.to_owned(),
                font_size: 64.0,
                color: [0, 0, 0, 255],
                layout: crate::Layout::default(),
                depth: None,
            },
            SlugTextFont(fid),
            MaterialNode(mat_handle),
        ))
        .id();

    (world, entity, fid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRA_TTF: &[u8] = include_bytes!("../tests/fixtures/FiraMono-Medium.ttf");

    #[test]
    fn slug_atlas_register_and_ensure() {
        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font failed");
        let r = atlas.validate_glyphs(fid, "hello");
        assert!(r.atlas_grew(), "first ensure must add glyphs");
        assert!(r.missing.is_empty());
    }

    #[test]
    fn slug_atlas_shape_after_frame_build() {
        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font failed");
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
        let fid = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font failed");
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

    /// Convenience fn run `sync_text_meshes_texture` only once on `world`.
    fn run_sync_once(world: &mut bevy::ecs::world::World) {
        use bevy::ecs::system::RunSystemOnce;
        world
            .run_system_once(sync_text_meshes_texture)
            .expect("sync_text_meshes_texture must run without error");
    }

    #[test]
    fn sync_text_meshes_texture_processes_entity_when_images_are_uninitialized() {
        use bevy::asset::AssetId;
        use bevy::prelude::Image;

        let (mut world, entity, _fid) = make_test_world_with_ui_entity(FIRA_TTF, "Hi");

        assert!(
            !world.resource::<AtlasDirtyFlag>().0,
            "flag must start false"
        );
        assert!(
            !world.contains_resource::<SlugAtlasNotReady>(),
            "SlugAtlasNotReady must not be present"
        );

        run_sync_once(&mut world);

        let mn = world
            .entity(entity)
            .get::<bevy::prelude::MaterialNode<SlugTextTextureMaterial>>()
            .expect("MaterialNode<SlugTextTextureMaterial> must be present")
            .0
            .clone();
        let mats = world.resource::<bevy::prelude::Assets<SlugTextTextureMaterial>>();
        let mat = mats.get(&mn).expect("material must exist in Assets");

        assert_ne!(
            mat.runs_image.id(),
            AssetId::<Image>::default(),
            "runs_image is still the default handle  sync_text_meshes_texture did not process the entity"
        );
        assert_ne!(
            mat.glyph_layout_image.id(),
            AssetId::<Image>::default(),
            "glyph_layout_image is still the default handle"
        );
    }

    #[test]
    fn sync_text_meshes_texture_ui_entity_does_not_get_mesh2d() {
        let (mut world, entity, _fid) = make_test_world_with_ui_entity(FIRA_TTF, "Hi");

        world.resource_mut::<AtlasDirtyFlag>().0 = true;
        run_sync_once(&mut world);

        assert!(
            world
                .entity(entity)
                .get::<bevy::prelude::Mesh2d>()
                .is_none(),
            "sync_text_meshes_texture inserted Mesh2d on a UI entity the mat_node branch must `continue` before the mesh path"
        );
    }

    /// `RENDER_WORLD` only images are dropped from CPU memory after the first
    /// GPU upload. Any subsequent render-world rebuild e.g. after a device
    /// loss or pipeline invalidation finds no data to re-upload, causing the
    /// text to silently disappear.
    #[test]
    fn sync_text_meshes_texture_ui_images_have_main_world_usage() {
        use bevy::asset::RenderAssetUsages;

        let (mut world, entity, _fid) = make_test_world_with_ui_entity(FIRA_TTF, "Hi");
        world.resource_mut::<AtlasDirtyFlag>().0 = true;
        run_sync_once(&mut world);

        let mn = world
            .entity(entity)
            .get::<bevy::prelude::MaterialNode<SlugTextTextureMaterial>>()
            .expect("MaterialNode<SlugTextTextureMaterial> must be present")
            .0
            .clone();
        let mats = world.resource::<bevy::prelude::Assets<SlugTextTextureMaterial>>();
        let mat = mats.get(&mn).expect("material must exist");
        let images = world.resource::<bevy::prelude::Assets<bevy::prelude::Image>>();

        for (label, handle) in [
            ("runs_image", &mat.runs_image),
            ("glyph_layout_image", &mat.glyph_layout_image),
        ] {
            let img = images.get(handle).unwrap_or_else(|| {
                panic!(
                    "{label} is not in Assets<Image> handle is still default or image was never inserted"
                )
            });
            assert!(
                img.asset_usage.contains(RenderAssetUsages::MAIN_WORLD),
                "{label} has usage {:?} MAIN_WORLD is required",
                img.asset_usage
            );
        }
    }
}
