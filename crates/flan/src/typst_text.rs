/// Bevy integration for typst-sourced 3-D text rendering.
///
/// # Pipeline
///
/// ```text
///  Resource change / spawn with TypstDirty
///              │
///              ▼
///  ┌───────────────────────┐
///  │       TypstDirty      │  ← entity needs to be rebuilt
///  └───────────┬───────────┘
///              │
///              │  typst_prepare, runs before SlugAtlasSet
///              │  • layout_typst -> validate_glyph_ids
///              │  • push glyph IDs -> ExtraGlyphNeeds
///              │  • store spans on entity -> TypstShapeCache  (deferred)
///              │
///              │  ┌─────────────────────────────────────────────────────┐
///              │  │  SlugAtlasSet                                       │
///              │  │  build_frame_atlas -> upload_atlas -> sync_meshes     │
///              │  └─────────────────────────────────────────────────────┘
///              │
///              │  typst_materialize  runs after SlugAtlasSet
///              │  • read TypstShapeCache written this frame by typst_prepare
///              │  • build_run_from_spans
///              │
///       ┌──────┴──────┐
///       │ atlas miss? │
///       └──────┬──────┘
///    yes │          │ no
///        │          ▼
///        │    build Mesh + Material
///        │    remove TypstDirty  deferred to later
///        │          │
///        │          ▼
///        │  ┌───────────────────────────────┐
///        │  │  Mesh3d + MeshMaterial3d      │
///        │  │        + Text3dDirty          │
///        │  └───────────────┬───────────────┘
///        │                  │
///        │                  │  typst_clear_dirty  runs after SlugAtlasSet
///        │                  │  • removes Text3dDirty once material
///        │                  │    atlas handles are populated
///        │                  │
///        │                  ▼
///        │  ┌───────────────────────────────┐
///        │  │  Mesh3d + MeshMaterial3d      │  ← stable, rendering
///        │  └───────────────────────────────┘
///        │
///        │ stay TypstDirty
///        └──────────────────► re-push glyph IDs next frame
/// ```
///
/// Non-dirty rendered entities keep their [`TypstShapeCache`] and re-push
/// glyph IDs to [`ExtraGlyphNeeds`] every frame so the frame atlas never
/// evicts their glyphs. This is intentional for now.
use bevy::prelude::*;

use crate::slug::{FontId, SlugAtlas, SlugGlyphLayout, SlugTextRun, SlugVertex};
use crate::slug_text::{
    ExtraGlyphNeeds, SlugAtlasSet, Text3dDirty, build_mesh_from_run, normalize_run_3d,
};
use crate::slug_typst::{TypstGlyphSpan, layout_typst};

#[cfg(not(feature = "webgl"))]
use crate::slug_text_material::SlugText3dMaterial;
#[cfg(feature = "webgl")]
use crate::slug_text_material::SlugText3dTextureMaterial;

/// Drives typst-sourced 3-D text rendering for an entity.
#[derive(Component, Clone, Debug)]
pub struct TypstTextNode {
    /// Typst markup source.
    pub source: String,
    /// Font registered with [`SlugAtlas`].
    pub font_id: FontId,
    /// Pixel size per typst point (`1.0` -> 1 pt = 1 px).
    pub pixels_per_pt: f32,
    /// RGBA8 text color.
    pub color: [u8; 4],
}

impl Default for TypstTextNode {
    fn default() -> Self {
        Self {
            source: String::new(),
            font_id: FontId(0),
            pixels_per_pt: 1.5,
            color: [255, 255, 255, 255],
        }
    }
}

/// Marker: this entity needs to be reshaped and mesh rebuilt.
#[derive(Component, Default)]
pub struct TypstDirty;

/// Convenience to spawn a typst text entity ready for the pipeline.
#[derive(Bundle)]
pub struct TypstTextMesh {
    pub node: TypstTextNode,
    pub dirty: TypstDirty,
    pub mesh: Mesh3d,
    pub transform: Transform,
    pub visibility: Visibility,
}

impl Default for TypstTextMesh {
    fn default() -> Self {
        Self {
            node: TypstTextNode::default(),
            dirty: TypstDirty,
            mesh: Mesh3d::default(),
            transform: Transform::default(),
            visibility: Visibility::default(),
        }
    }
}

/// Shaped glyph data kept on the entity across frames.
#[derive(Component)]
struct TypstShapeCache {
    spans: Vec<TypstGlyphSpan>,
    // Deduped, for ExtraGlyphNeeds re-push data
    glyph_ids: Vec<u16>,
    font_id: FontId,
}

/// Same-frame span bridge: written immediately which is not deferred by
/// [`typst_prepare`] and drained by [`typst_materialize`] within the same
/// `Update` run.
#[derive(Resource, Default)]
struct TypstPendingSpans(
    std::collections::HashMap<Entity, (Vec<TypstGlyphSpan>, Vec<u16>, FontId)>,
);

/// Registers the typst text pipeline systems.
pub struct TypstTextPlugin;

impl Plugin for TypstTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TypstPendingSpans>();
        app.add_systems(Update, typst_prepare.before(SlugAtlasSet));
        app.add_systems(Update, typst_materialize.after(SlugAtlasSet));
        app.add_systems(Update, typst_clear_dirty.after(SlugAtlasSet));
    }
}

/// Shape dirty entities and keep non-dirty ones alive in the atlas.
/// Runs prior to [`SlugAtlasSet`] every frame.
///
/// On atlas miss in [`typst_materialize`] the entity keeps [`TypstDirty`], so
/// this system re-shapes and re-pushes the next frame automatic retry with no
/// extra bookkeeping.
#[allow(clippy::type_complexity)]
fn typst_prepare(
    mut atlas: ResMut<SlugAtlas>,
    mut extra_needs: ResMut<ExtraGlyphNeeds>,
    mut pending: ResMut<TypstPendingSpans>,
    mut commands: Commands,
    // Dirty entities: always (re)shape.
    dirty_q: Query<(Entity, &TypstTextNode), With<TypstDirty>>,
    // Rendered (non-dirty) entities: keep-alive only.
    alive_q: Query<&TypstShapeCache, (With<TypstTextNode>, Without<TypstDirty>)>,
) {
    // Clear last frame's pending spans.
    pending.0.clear();

    let n_alive = alive_q.iter().count();
    let n_dirty = dirty_q.iter().count();

    bevy::log::trace!(
        "typst_prepare: {} alive (keep-alive), {} dirty (need shape)",
        n_alive,
        n_dirty
    );

    for cache in alive_q.iter() {
        extra_needs.0.push((cache.font_id, cache.glyph_ids.clone()));
    }

    for (entity, node) in dirty_q.iter() {
        bevy::log::debug!(
            "typst_prepare: {:?} shaping source len={} font={:?} ppp={}",
            entity,
            node.source.len(),
            node.font_id,
            node.pixels_per_pt
        );

        if node.source.is_empty() {
            bevy::log::debug!("typst_prepare: {:?} source empty, skipping", entity);
            continue;
        }

        let font_bytes = match atlas.font_bytes(node.font_id) {
            Some(b) => b.to_vec(),
            None => {
                bevy::log::warn!(
                    "typst_prepare: {:?} FontId {:?} not in SlugAtlas, skipping",
                    entity,
                    node.font_id
                );
                continue;
            }
        };

        let spans = layout_typst(&node.source, &font_bytes);
        bevy::log::debug!(
            "typst_prepare: {:?} layout_typst produced {} span(s)",
            entity,
            spans.len()
        );

        if spans.is_empty() {
            bevy::log::warn!(
                "typst_prepare: {:?} zero spans; source preview: {:?}",
                entity,
                &node.source[..node.source.len().min(80)]
            );
            continue;
        }

        let mut glyph_ids: Vec<u16> = spans.iter().map(|s| s.glyph_id).collect();
        glyph_ids.sort_unstable();
        glyph_ids.dedup();

        let validate = atlas.validate_glyph_ids(node.font_id, &glyph_ids);
        bevy::log::debug!(
            "typst_prepare: {:?} {} unique glyph(s); newly_added={} missing={}",
            entity,
            glyph_ids.len(),
            validate.newly_added.len(),
            validate.missing.len()
        );

        extra_needs.0.push((node.font_id, glyph_ids.clone()));

        let font_id = node.font_id;
        pending
            .0
            .insert(entity, (spans.clone(), glyph_ids.clone(), font_id));
        bevy::log::debug!(
            "typst_prepare: {:?} spans written to TypstPendingSpans",
            entity
        );

        // Also persist shapes on the entity component for future frames.
        commands.queue(move |world: &mut World| {
            if let Ok(mut e) = world.get_entity_mut(entity) {
                e.insert(TypstShapeCache {
                    spans,
                    glyph_ids,
                    font_id,
                });
            } else {
                bevy::log::debug!(
                    "typst_prepare: {:?} entity despawned before TypstShapeCache insert",
                    entity
                );
            }
        });
    }
}

/// Build meshes from cached spans that runs after [`SlugAtlasSet`].
#[allow(clippy::type_complexity)]
fn typst_materialize(
    atlas: Res<SlugAtlas>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    pending: Res<TypstPendingSpans>,
    dirty_q: Query<
        (
            Entity,
            &TypstTextNode,
            Option<&TypstShapeCache>,
            Option<&Mesh3d>,
        ),
        With<TypstDirty>,
    >,
) {
    let n = dirty_q.iter().count();
    if n > 0 {
        bevy::log::debug!("typst_materialize: {} dirty entity/entities to process", n);
    }

    for (entity, node, maybe_cache, current_mesh) in dirty_q.iter() {
        let (spans, font_id) = if let Some((spans, _, fid)) = pending.0.get(&entity) {
            bevy::log::debug!(
                "typst_materialize: {:?} using pending spans ({} span(s)) from this frame",
                entity,
                spans.len()
            );
            (spans.as_slice(), *fid)
        } else if let Some(cache) = maybe_cache {
            bevy::log::debug!(
                "typst_materialize: {:?} using cached spans ({} span(s)) from prior frame",
                entity,
                cache.spans.len()
            );
            (cache.spans.as_slice(), cache.font_id)
        } else {
            bevy::log::debug!(
                "typst_materialize: {:?} no spans available yet (will retry next frame)",
                entity
            );
            continue;
        };

        let run = match build_run_from_spans(&atlas, font_id, spans, node.pixels_per_pt, node.color)
        {
            Some(r) => r,
            None => {
                bevy::log::debug!(
                    "typst_materialize: {:?} atlas miss, will retry next frame",
                    entity
                );
                continue;
            }
        };

        if run.is_empty() {
            bevy::log::debug!(
                "typst_materialize: {:?} run empty (all whitespace?), skipping",
                entity
            );
            continue;
        }

        bevy::log::debug!(
            "typst_materialize: {:?} run ok: {} vert(s), {} glyph(s), \
             advance={:.1} height={:.1}",
            entity,
            run.vertices.len(),
            run.glyph_layout.len(),
            run.natural_advance,
            run.natural_height
        );

        let inv_h = 1.0 / run.natural_height.max(f32::EPSILON);
        let normalized = normalize_run_3d(&run, inv_h);
        let new_mesh = build_mesh_from_run(&normalized, None);

        let new_handle = match current_mesh {
            Some(m) if m.id() != bevy::asset::AssetId::default() => {
                if let Some(mut slot) = meshes.get_mut(m.id()) {
                    *slot = new_mesh;
                    m.0.clone()
                } else {
                    meshes.add(new_mesh)
                }
            }
            _ => meshes.add(new_mesh),
        };

        #[cfg(not(feature = "webgl"))]
        let text_color = {
            let [cr, cg, cb, ca] = node.color;
            bevy::math::Vec4::new(
                cr as f32 / 255.0,
                cg as f32 / 255.0,
                cb as f32 / 255.0,
                ca as f32 / 255.0,
            )
        };

        // Insert components and remove TypstDirty so the operation is skipped
        // on despawn.
        commands.queue(move |world: &mut World| {
            if world.get_entity(entity).is_err() {
                bevy::log::debug!(
                    "typst_materialize: {:?} despawned before material insert",
                    entity
                );
                return;
            }

            #[cfg(not(feature = "webgl"))]
            {
                use bevy::asset::RenderAssetUsages;
                use bevy::render::render_resource::BufferUsages;

                // Pre-fill the params buffer so the text is visible
                // immediately.
                let params = {
                    let mut bufs = world
                        .get_resource_mut::<Assets<bevy::render::storage::ShaderBuffer>>()
                        .expect("Assets<ShaderBuffer> must exist");
                    let mut initial = [0u8; 96];
                    // offset 16: text_color vec4
                    initial[16..32].copy_from_slice(bytemuck::bytes_of(&text_color));
                    // offset 32: local_to_clip start with identity
                    let identity = bevy::math::Mat4::IDENTITY.to_cols_array_2d();
                    initial[32..96].copy_from_slice(bytemuck::cast_slice(&identity));
                    let mut ssb = bevy::render::storage::ShaderBuffer::new(
                        &initial,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    ssb.buffer_description.usage = BufferUsages::UNIFORM | BufferUsages::COPY_DST;
                    bufs.add(ssb)
                };

                // Grab the current shared atlas handles immediately so the
                // material is fully bound from frame 1. upload_atlas_system
                // runs inside SlugAtlasSet and by the time this executes the
                // atlas handles are already populated in SlugAtlasBuffers for
                // this frame.
                let (curves, curve_indices, glyphs) = {
                    let ab = world
                        .get_resource::<crate::SlugAtlasBuffers>()
                        .expect("SlugAtlasBuffers must exist");
                    (
                        ab.curves.clone(),
                        ab.curve_indices.clone(),
                        ab.glyphs.clone(),
                    )
                };

                bevy::log::debug!(
                    "typst_materialize: {:?} atlas handles: curves={:?} ci={:?} glyphs={:?}",
                    entity,
                    curves.id() != bevy::asset::AssetId::default(),
                    curve_indices.id() != bevy::asset::AssetId::default(),
                    glyphs.id() != bevy::asset::AssetId::default(),
                );

                let mat = {
                    let mut mats = world
                        .get_resource_mut::<Assets<SlugText3dMaterial>>()
                        .expect("Assets<SlugText3dMaterial> must exist");
                    mats.add(SlugText3dMaterial {
                        params_buf: Some(params),
                        text_color,
                        curves,
                        curve_indices,
                        glyphs,
                        ..Default::default()
                    })
                };

                if let Ok(mut e) = world.get_entity_mut(entity) {
                    e.insert(Mesh3d(new_handle));
                    e.insert(MeshMaterial3d(mat));
                    // Text3dDirty tells upload_atlas_system to keep running
                    e.insert(Text3dDirty);
                    e.remove::<TypstDirty>();
                    bevy::log::debug!(
                        "typst_materialize: {:?} Mesh3d+Material+Text3dDirty inserted, \
                         TypstDirty removed (native)",
                        entity
                    );
                } else {
                    bevy::log::debug!(
                        "typst_materialize: {:?} despawned before material insert",
                        entity
                    );
                }
            }

            #[cfg(feature = "webgl")]
            {
                let (curves_image, curve_indices_image, glyphs_image) = {
                    let ai = world
                        .get_resource::<crate::slug_text_material::SlugAtlasImages>()
                        .expect("SlugAtlasImages must exist");
                    (
                        ai.curves.clone(),
                        ai.curve_indices.clone(),
                        ai.glyphs.clone(),
                    )
                };

                bevy::log::debug!(
                    "typst_materialize: {:?} atlas image handles: curves={:?} ci={:?} glyphs={:?}",
                    entity,
                    curves_image.id() != bevy::asset::AssetId::default(),
                    curve_indices_image.id() != bevy::asset::AssetId::default(),
                    glyphs_image.id() != bevy::asset::AssetId::default(),
                );

                let mat = {
                    let mut mats = world
                        .get_resource_mut::<Assets<SlugText3dTextureMaterial>>()
                        .expect("Assets<SlugText3dTextureMaterial> must exist");
                    mats.add(SlugText3dTextureMaterial {
                        curves_image,
                        curve_indices_image,
                        glyphs_image,
                        ..Default::default()
                    })
                };

                if let Ok(mut e) = world.get_entity_mut(entity) {
                    e.insert(Mesh3d(new_handle));
                    e.insert(MeshMaterial3d(mat));
                    e.insert(Text3dDirty);
                    e.remove::<TypstDirty>();
                    bevy::log::debug!(
                        "typst_materialize: {:?} Mesh3d+Material+Text3dDirty inserted, \
                         TypstDirty removed (webgl)",
                        entity
                    );
                } else {
                    bevy::log::debug!(
                        "typst_materialize: {:?} despawned before material insert",
                        entity
                    );
                }
            }
        });
    }
}

/// Remove [`Text3dDirty`] once `upload_atlas_system` has populated the
/// material's atlas buffer / image handles.
#[allow(clippy::type_complexity)]
fn typst_clear_dirty(
    mut commands: Commands,
    #[cfg(not(feature = "webgl"))] dirty_q: Query<
        (Entity, &MeshMaterial3d<SlugText3dMaterial>),
        (With<TypstTextNode>, With<Text3dDirty>),
    >,
    #[cfg(not(feature = "webgl"))] mat_assets: Res<Assets<SlugText3dMaterial>>,
    #[cfg(feature = "webgl")] dirty_q: Query<
        (Entity, &MeshMaterial3d<SlugText3dTextureMaterial>),
        (With<TypstTextNode>, With<Text3dDirty>),
    >,
    #[cfg(feature = "webgl")] mat_assets: Res<Assets<SlugText3dTextureMaterial>>,
) {
    for (entity, mm) in dirty_q.iter() {
        let ready = mat_assets.get(&mm.0).is_some_and(|mat| {
            #[cfg(not(feature = "webgl"))]
            {
                mat.curves.id() != bevy::asset::AssetId::default()
            }
            #[cfg(feature = "webgl")]
            {
                mat.curves_image.id() != bevy::asset::AssetId::default()
            }
        });

        if ready {
            commands.queue(move |world: &mut World| {
                if let Ok(mut e) = world.get_entity_mut(entity) {
                    e.remove::<Text3dDirty>();
                    bevy::log::debug!(
                        "typst_clear_dirty: {:?} atlas handles populated, \
                         Text3dDirty removed -> fully stable",
                        entity
                    );
                }
            });
        } else {
            bevy::log::debug!(
                "typst_clear_dirty: {:?} waiting for atlas SSB handles \
                 (curves handle still default)",
                entity
            );
        }
    }
}

/// Convert [`TypstGlyphSpan`]s into a [`SlugTextRun`].
///
/// Returns `None` if any glyph is missing from the current frame atlas
/// as an atlas miss, the caller should leave the entity dirty for retry.
fn build_run_from_spans(
    atlas: &SlugAtlas,
    font_id: FontId,
    spans: &[TypstGlyphSpan],
    pixels_per_pt: f32,
    color: [u8; 4],
) -> Option<SlugTextRun> {
    let (ascender_fu, _descender_fu) = atlas.font_metrics(font_id)?;
    let upm = atlas.units_per_em(font_id)? as f32;

    let mut vertices: Vec<SlugVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut glyph_layout: Vec<SlugGlyphLayout> = Vec::new();
    let mut max_x_px: f32 = 0.0;
    let mut max_y_px: f32 = 0.0;
    let mut atlas_misses: usize = 0;

    for span in spans {
        let gid = span.glyph_id;

        // Check outline data first. Whitespace and other no-outline glyphs
        // are never added to the frame atlas, so checking glyph_index first
        // would spuriously increment atlas_misses for every space character.
        let cached = match atlas.cached_glyph(font_id, gid) {
            Some(c) => c,
            None => continue, // whitespace / no outline skip silently, not a miss just no shape data
        };

        // Glyph has outline data so it must be present in the frame atlas.
        let abs_idx = match atlas.frame.glyph_index(font_id, gid) {
            Some(i) => i,
            None => {
                atlas_misses += 1;
                continue;
            }
        };

        let scale = (span.font_size_pt as f32 * pixels_per_pt) / upm;
        let asc_px = ascender_fu * scale;
        let ox = span.x_pt as f32 * pixels_per_pt;
        let by = span.y_pt as f32 * pixels_per_pt;

        let [xmin_e, ymin_e, xmax_e, ymax_e] = cached.bbox.map(|v| v as f32);
        let xmin = ox + xmin_e * scale;
        let xmax = ox + xmax_e * scale;
        let ymin = by + asc_px - ymax_e * scale;
        let ymax = by + asc_px - ymin_e * scale;

        if xmax > max_x_px {
            max_x_px = xmax;
        }
        if ymax > max_y_px {
            max_y_px = ymax;
        }

        let base = vertices.len() as u32;
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);

        #[inline(always)]
        fn pack(hi: i16, lo: i16) -> u32 {
            ((hi as u16 as u32) << 16) | (lo as u16 as u32)
        }
        let [xi, yi, xa, ya] = cached.bbox;
        vertices.push(SlugVertex {
            pos: [xmin, ymin, -1.0, 1.0],
            glyph: [pack(xi, ya), abs_idx],
            color,
        });
        vertices.push(SlugVertex {
            pos: [xmax, ymin, 1.0, 1.0],
            glyph: [pack(xa, ya), abs_idx],
            color,
        });
        vertices.push(SlugVertex {
            pos: [xmin, ymax, -1.0, -1.0],
            glyph: [pack(xi, yi), abs_idx],
            color,
        });
        vertices.push(SlugVertex {
            pos: [xmax, ymax, 1.0, -1.0],
            glyph: [pack(xa, yi), abs_idx],
            color,
        });

        glyph_layout.push(SlugGlyphLayout {
            screen_rect: [xmin, ymin, xmax, ymax],
            em_rect: [xi as f32, ya as f32, xa as f32, yi as f32],
            glyph_index: abs_idx,
            _pad: [0; 3],
        });
    }

    if atlas_misses > 0 {
        bevy::log::debug!(
            "build_run_from_spans: font {:?} {}/{} glyph(s) not in frame atlas yet",
            font_id,
            atlas_misses,
            spans.len()
        );
        return None;
    }

    if vertices.is_empty() {
        return None;
    }

    Some(SlugTextRun {
        vertices,
        indices,
        glyph_layout,
        font_id,
        natural_advance: max_x_px,
        // Use the actual pixel bounding box height of all rendered spans so
        // that multi-line typst paragraphs normalize correctly.
        natural_height: max_y_px.max(f32::EPSILON),
    })
}
