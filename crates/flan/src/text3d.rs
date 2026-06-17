//! 3d text entity lifecycle plugin.
//!
//! # Single-resource state model
//!
//! [`SlugText3dState`] is the source for every aspect of the 3d text subsystem.
//! Any system that wants to change the text, color, renderer, depth,
//! visibility, or font writes to this one resource.
//!
//! [`debounce_slug_text3d`] watches `SlugText3dState::is_changed()`, resets a
//! `Local<Option<Timer>>` on each write, and fires [`SlugText3dApply`] once the
//! state has been quiet for `DEBOUNCE_MS` ms.
//!
//! [`apply_slug_text3d`] consumes [`SlugText3dApply`] messages, diffs new state
//! against [`SlugText3dApplied`], and either despawns+respawns (renderer /
//! depth / visibility / font changes) or patches [`SlugTextNode`] in-place for
//! text or color. Every entity is spawned with
//! [`crate::slug_text::Text3dDirty`] and every in-place patch re-inserts it, so
//! `sync_text_meshes` retries the entity each frame until the material is ready
//! and the mesh has been built.
//!
//! Both native and WebGL render paths share this single code path.
//!
//! # Public surface
//!
//! | Item | Kind | Purpose |
//! |------|------|---------|
//! | [`SlugText3dState`] | Resource | desired state - write here |
//! | [`SlugText3dFontValidation`] | Resource | last font-validation result |
//! | [`Text3dRenderer`] | enum | flat vs extruded (inside State) |
//! | [`Text3d`] | Component | marker on the live entity |
//! | [`ShowText3d`] | Component | kept for API compat; on the Text3d entity |
//! | [`SlugTextAnchor`] | Component | recentering anchor for flat text |
//! | [`crate::slug_text::Text3dDirty`] | Component | pending upload marker; removed on success |
//! | [`spawn_text3d`] | fn | low-level spawn helper |
//! | [`SlugText3dPlugin`] | Plugin | registers everything |

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;

use crate::{SlugTextFont, SlugTextMesh, SlugTextNode, slug::SlugAtlas};

const DEFAULT_DEPTH: f32 = 0.06;

/// How long the state must be quiet before [`SlugText3dApply`] is fired.
const DEBOUNCE_MS: u64 = 150;

/// Which render path is used for the 3d text label.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text3dRenderer {
    /// Flat / decal slug text in world space (`depth = None`).
    SlugText,
    /// Extruded slug text: earcutr-triangulated caps + side walls via
    /// `StandardMaterial` (`depth = Some(...)`).
    #[default]
    SlugText3d,
}

/// Write to any field from egui, reverie-sync, or any other system.
/// [`debounce_slug_text3d`] will coalesce rapid writes and fire
/// [`SlugText3dApply`] once the state has been stable for `DEBOUNCE_MS` ms.
///
/// # Field semantics
///
/// - `visible`      - whether a [`Text3d`] entity should exist at all
/// - `renderer`     - flat vs extruded; change triggers a full respawn
/// - `depth`        - extrusion depth (SlugText3d only); change triggers respawn
/// - `color`        - RGBA8; patched in-place, no respawn
/// - `text`         - the live string; patched in-place, no respawn
/// - `default_text` - fallback shown when nothing else overrides `text`
/// - `font_id`      - which registered font to use; validated on apply
#[derive(Resource, Clone, Debug)]
pub struct SlugText3dState {
    pub visible: bool,
    pub renderer: Text3dRenderer,
    pub depth: f32,
    pub color: [u8; 4],
    /// Live text currently displayed. Overwritten by reverie sync each second.
    pub text: String,
    /// User-editable fallback text (egui text box). Used by
    /// `sync_text3d_to_active_reverie` when there is no active content.
    pub default_text: String,
    /// Font to use. `None` means the font has not been loaded yet; the apply
    /// system will skip spawning until this is `Some`.
    pub font_id: Option<crate::slug::FontId>,
}

const DEFAULT_TEXT: &str = "mitchty 美智君";

impl Default for SlugText3dState {
    fn default() -> Self {
        Self {
            visible: false,
            renderer: Text3dRenderer::default(),
            depth: DEFAULT_DEPTH,
            color: [0, 0, 0, 255],
            text: String::from(DEFAULT_TEXT),
            default_text: String::from(DEFAULT_TEXT),
            font_id: None,
        }
    }
}

/// Bevy Message fired by the debounce system after [`SlugText3dState`] has been
/// quiet for `DEBOUNCE_MS` milliseconds.
///
/// Any system can read this to react when the 3d text state settles.
/// The core apply logic is handled automatically by [`apply_slug_text3d`].
#[derive(Clone, Copy, Debug)]
pub struct SlugText3dApply;
impl bevy::ecs::message::Message for SlugText3dApply {}

/// Outcome of the most recent font-validation attempt.
///
/// `missing_glyphs` is empty when the last attempt succeeded or no attempt has
/// been made yet.
#[derive(Resource, Default, Clone, Debug)]
pub struct SlugText3dFontValidation {
    pub missing_glyphs: Vec<char>,
}

/// Internal snapshot of the last state that was successfully applied.
///
/// Compared against [`SlugText3dState`] in [`apply_slug_text3d`] to determine
/// whether a respawn or an in-place patch is needed.
#[derive(Resource, Default, Clone)]
pub struct SlugText3dApplied(Option<SlugText3dState>);

/// Marker on the live [`Text3d`] entity.
#[derive(Component)]
pub struct Text3d;

/// Marker kept for API compatibility. Inserted on the [`Text3d`] entity when
/// visible; removed when hidden. Queries on `With<ShowText3d>` continue to
/// work without changes.
#[derive(Component)]
pub struct ShowText3d;

/// Stores the intended world-space center for a flat [`Text3d`] entity so that
/// [`center_slug_text`] can re-anchor it after the `Aabb` is computed.
#[derive(Component)]
pub struct SlugTextAnchor(pub Vec3);

/// Watch [`SlugText3dState`] for changes and write a [`SlugText3dApply`]
/// message after the state has been quiet for `DEBOUNCE_MS` milliseconds.
pub fn debounce_slug_text3d(
    state: Res<SlugText3dState>,
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    mut msg: bevy::ecs::message::MessageWriter<SlugText3dApply>,
) {
    if state.is_changed() {
        *timer = Some(Timer::from_seconds(
            DEBOUNCE_MS as f32 / 1000.0,
            TimerMode::Once,
        ));
    }
    let Some(t) = timer.as_mut() else { return };
    t.tick(time.delta());
    if t.just_finished() {
        msg.write(SlugText3dApply);
        *timer = None;
    }
}

/// Read [`SlugText3dApply`] messages, diff new state vs last-applied snapshot,
/// and either respawn or patch in-place.
///
/// Decision table, too complex need to rethink this to simplify:
///
/// | Change | Action |
/// |--------|--------|
/// | `visible` false | despawn all `Text3d` entities |
/// | `renderer` or `depth` changed, or no entity | despawn + respawn |
/// | `font_id` loaded for first time | respawn (font now available) |
/// | `text` or `color` changed only | patch `SlugTextNode` in-place |
/// | `font_id` changed (already had one) | validate + patch `SlugTextFont` |
#[allow(clippy::too_many_arguments)]
pub fn apply_slug_text3d(
    mut msg: bevy::ecs::message::MessageReader<SlugText3dApply>,
    state: Res<SlugText3dState>,
    mut applied: ResMut<SlugText3dApplied>,
    existing: Query<Entity, With<Text3d>>,
    mut text3d_nodes: Query<(&mut SlugTextNode, &mut SlugTextFont), With<Text3d>>,
    mut atlas: ResMut<SlugAtlas>,
    mut validation: ResMut<SlugText3dFontValidation>,
    mut font_id_res: Option<ResMut<super::text3d_font::Text3dFontId>>,
    mut commands: Commands,
) {
    if msg.read().count() == 0 {
        return;
    }

    let new = state.clone();
    let old = applied.0.clone();

    let existing_entities: Vec<Entity> = existing.iter().collect();
    let has_entity = !existing_entities.is_empty();

    if !new.visible {
        for e in &existing_entities {
            commands.entity(*e).despawn();
        }
        applied.0 = Some(new);
        return;
    }

    if new.font_id.is_none() {
        // Don't update applied so the next write (font arriving) retriggers.
        return;
    }

    let renderer_changed = old
        .as_ref()
        .map(|o| o.renderer != new.renderer)
        .unwrap_or(true);
    let depth_changed = old
        .as_ref()
        .map(|o| (o.depth - new.depth).abs() > f32::EPSILON)
        .unwrap_or(true);
    let became_visible = old.as_ref().map(|o| !o.visible).unwrap_or(true);
    let font_was_none = old.as_ref().map(|o| o.font_id.is_none()).unwrap_or(true);

    let needs_respawn =
        !has_entity || renderer_changed || depth_changed || became_visible || font_was_none;

    if needs_respawn {
        for e in &existing_entities {
            commands.entity(*e).despawn();
        }
        spawn_text3d(
            &mut commands,
            new.font_id.map(super::text3d_font::Text3dFontId).as_ref(),
            &new.text,
            new.renderer,
            new.depth,
            new.color,
        );
        applied.0 = Some(new);
        return;
    }

    let font_changed = old
        .as_ref()
        .map(|o| o.font_id != new.font_id)
        .unwrap_or(false);

    for (mut node, mut slug_font) in text3d_nodes.iter_mut() {
        node.text = new.text.clone();
        node.color = new.color;

        if font_changed && let Some(font_id) = new.font_id {
            let result = atlas.validate_glyphs(font_id, &new.text);
            if result.missing.is_empty() {
                slug_font.0 = font_id;
                if let Some(ref mut res) = font_id_res {
                    res.0 = font_id;
                }
                validation.missing_glyphs.clear();
            } else {
                validation.missing_glyphs = result.missing;
            }
        }
    }

    // Mark all Text3d entities dirty so sync_text_meshes re-uploads them even
    // after their Changed<SlugTextNode> detector has expired (e.g. if the
    // material wasn't ready on the frame the node was mutated).
    for e in &existing_entities {
        commands.entity(*e).insert(crate::slug_text::Text3dDirty);
    }

    applied.0 = Some(new);
}

/// Recenter flat [`Text3d`] entities once the mesh `Aabb` has been computed.
///
/// Runs in `PostUpdate` after `CalculateBounds`.
#[allow(clippy::type_complexity)]
pub fn center_slug_text(
    mut query: Query<(&mut Transform, &Aabb, &SlugTextAnchor), (With<Text3d>, Changed<Aabb>)>,
) {
    for (mut transform, aabb, anchor) in query.iter_mut() {
        transform.translation.x = anchor.0.x - transform.scale.x * aabb.center.x;
        transform.translation.y = anchor.0.y - transform.scale.y * aabb.center.y;
    }
}

/// Spawn a [`Text3d`] entity using the given renderer.
///
/// The correct material is inserted automatically by `init_slug_entity`.
/// Callers never create materials manually.
pub fn spawn_text3d(
    commands: &mut Commands,
    flan_font: Option<&super::text3d_font::Text3dFontId>,
    text: &str,
    renderer: Text3dRenderer,
    depth: f32,
    color: [u8; 4],
) {
    let transform = Transform::from_xyz(0.0, 0.7, 0.0).with_scale(Vec3::splat(0.5));

    let Some(font) = flan_font else {
        bevy::log::warn!("spawn_text3d: font not ready yet, skipping spawn");
        return;
    };

    let node_depth = match renderer {
        Text3dRenderer::SlugText => None,
        Text3dRenderer::SlugText3d => Some(depth),
    };

    commands.spawn((
        SlugTextMesh {
            node: SlugTextNode {
                text: text.to_string(),
                color,
                depth: node_depth,
                ..default()
            },
            font: SlugTextFont(font.0),
            transform,
            ..default()
        },
        SlugTextAnchor(transform.translation),
        ShowText3d,
        Text3d,
        crate::slug_text::Text3dDirty,
    ));
}

/// Bevy plugin that owns the 3d text entity lifecycle.
///
/// Add this plugin, then write to [`SlugText3dState`] from your application
/// systems to control what is displayed.
pub struct SlugText3dPlugin;

impl Plugin for SlugText3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SlugText3dState>()
            .init_resource::<SlugText3dApplied>()
            .init_resource::<SlugText3dFontValidation>()
            .add_message::<SlugText3dApply>();

        app.add_systems(Update, debounce_slug_text3d)
            // apply_slug_text3d must run BEFORE init_slug_entity so that any
            // entity spawned here (via Commands) has its commands flushed and
            // its Added<SlugTextNode> detected in the same frame. Without
            // this ordering the new entity doesn't appear until the next frame
            // and the toggle between renderers looks like it does nothing.
            .add_systems(
                Update,
                apply_slug_text3d
                    .after(debounce_slug_text3d)
                    .before(crate::slug_text::init_slug_entity),
            )
            .add_systems(
                PostUpdate,
                center_slug_text.after(VisibilitySystems::CalculateBounds),
            );
    }
}
