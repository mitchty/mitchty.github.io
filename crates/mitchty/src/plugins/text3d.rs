//! 3-D text plugin.
//!
//! Owns the two-phase commit system that debounces text updates to avoid
//! churning mesh entities, the renderer selector, the slug-text centering
//! system, and the sync from the active reverie world-clock alarm systems.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;

use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName};

// TODO: Need a refactor step at some point to make fonts bevy asset handles and
// not embed them like this hack. That is a future mitc problem though. Current
// mitch is too lazy to care.

/// NotoSansJP to match what bevy slugtext was using prior to this.
const NOTO_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf");

/// The string currently displayed as 3-D text plus the user-configured default.
#[derive(Resource)]
pub struct Text3dContent {
    /// String currently being displayed.
    pub text: String,
    /// Default text shown when nothing is programmatically chosen.
    pub default_text: String,
}

impl Default for Text3dContent {
    fn default() -> Self {
        Self {
            text: String::from("mitchty.github.io"),
            default_text: String::from("mitchty.github.io"),
        }
    }
}

/// Staged default text waiting to be flushed into `Text3dContent`.
///
/// The UI writes here on every keystroke so the text field stays responsive.
/// A separate debounce system flushes the value only after 400 ms to avoid
/// churning mesh entities faster than can be spawned()
#[derive(Resource)]
pub struct Text3dDefaultPending(pub String);

impl Default for Text3dDefaultPending {
    fn default() -> Self {
        Self(String::from("mitchty.github.io"))
    }
}

/// Extrusion depth in world units used by the `SlugText3d` renderer.
///
/// This is the committed/in use value. Debounced to the material periodocally
/// like the text update approach.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Text3dDepth(pub f32);

impl Default for Text3dDepth {
    fn default() -> Self {
        Self(0.125)
    }
}

/// Staged depth value written by the UI on every drag event.
///
/// Mirrors `Text3dDefaultPending` logic, the ui writes here every frame/tick,
/// and `commit_pending_depth` flushes the value into `Text3dDepth` only after
/// `Text3dDepthCommitPending` has been present for 300 ms/debounce period,
/// giving time to finish the entity respawn before triggering another one
/// needlessly.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Text3dDepthPending(pub f32);

impl Default for Text3dDepthPending {
    fn default() -> Self {
        Self(0.125)
    }
}

/// Marker component spawned by `mark_depth_commit_pending` when
/// `Text3dDepthPending` changes. Consumed when `commit_pending_depth` once the
/// debounce timer fires.
#[derive(Component)]
pub struct Text3dDepthCommitPending;

/// Which renderer is used for the 3d text label.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text3dRenderer {
    /// Flat or decal slug text front/back face quad in world space.
    SlugText,
    /// Extruded slug text with front and back faces at +/- 1/2 defined world
    /// units with ide walls built from the glyphs slug bezier contours as an
    /// entity child.
    #[default]
    SlugText3d,
}

/// Holds `FontId` returned when the NotoSansJP font is registered with flan
/// `SlugAtlas`. Inserted at startup by `setup_flan_font`.
#[derive(Resource)]
pub struct FlanFontId(pub flan::slug::FontId);

/// Marker indicating there is text staged for sync into the 3-D text system.
#[derive(Component)]
pub struct Text3dCommitPending;

/// Marker for 3-D text mesh entities.
#[derive(Component)]
pub struct Text3d;

/// Stores the intended world-space center for a `SlugText` entity so that
/// `center_slug_text` can re-anchor after the `Aabb` is computed.
#[derive(Component)]
pub struct SlugTextAnchor(pub Vec3);

/// Marker controlling visibility of the 3-D text.
///
/// Spawn to show, despawn to hide. Nothing is despawned for now.
#[derive(Component)]
pub struct ShowText3d;

/// Startup system to register the NotoSansJP font with flan `SlugAtlas` and
/// insert a `FlanFontId` resource so `spawn_text3d` can reference it.
pub fn setup_flan_font(mut commands: Commands, mut atlas: ResMut<flan::slug::SlugAtlas>) {
    match atlas.register_font(NOTO_FONT.to_vec()) {
        Ok(id) => {
            commands.insert_resource(FlanFontId(id));
        }
        Err(e) => {
            bevy::log::error!("SlugText: failed to register NotoSansJP font: {e}");
        }
    }
}

/// Startup: spawn the `ShowText3d` marker so text is visible by default.
pub fn setup_3d_text(mut commands: Commands) {
    commands.spawn(ShowText3d);
}

/// Watches both pending staging resources and spawns their respective commit
/// marker components when a change arrives and no marker is currently present.
pub fn mark_pending_commits(
    text_pending: Res<Text3dDefaultPending>,
    depth_pending: Res<Text3dDepthPending>,
    text_marker_q: Query<(), With<Text3dCommitPending>>,
    depth_marker_q: Query<(), With<Text3dDepthCommitPending>>,
    mut commands: Commands,
) {
    if text_pending.is_changed() && text_marker_q.is_empty() {
        commands.spawn(Text3dCommitPending);
    }
    if depth_pending.is_changed() && depth_marker_q.is_empty() {
        commands.spawn(Text3dDepthCommitPending);
    }
}

/// Flushes the staged string into `Text3dContent` after the 400 ms debounce
/// timer fires and removes the `Text3dCommitPending` marker.
pub fn commit_pending_default(
    pending: Res<Text3dDefaultPending>,
    mut content: ResMut<Text3dContent>,
    marker: Query<Entity, With<Text3dCommitPending>>,
    mut commands: Commands,
) {
    content.default_text = pending.0.clone();
    content.text.clear();
    for entity in marker.iter() {
        commands.entity(entity).despawn();
    }
}

/// Flushes the staged depth into `Text3dDepth` after the debounce
/// timer fires and removes the `Text3dDepthCommitPending` marker.
pub fn commit_pending_depth(
    pending: Res<Text3dDepthPending>,
    mut depth: ResMut<Text3dDepth>,
    marker: Query<Entity, With<Text3dDepthCommitPending>>,
    mut commands: Commands,
) {
    // Only write through a DerefMut if the value actually changed
    if (depth.bypass_change_detection().0 - pending.0).abs() > f32::EPSILON {
        depth.0 = pending.0;
    }
    for entity in marker.iter() {
        commands.entity(entity).despawn();
    }
}

/// Updates `Text3dContent` every second based on active world-clock alarms and
/// the currently selected reverie, falling back to the default text.
///
/// Priority order:
/// 1. Countdown to the soonest active (non-expired) world-clock alarm.
/// 2. Display name of the currently active reverie.
/// 3. The user-configured default text.
pub fn sync_text3d_to_active_reverie(
    active_post: Res<ActiveReverie>,
    display_q: Query<&ReverieDisplayName>,
    world_clock: Option<Res<crate::ui::WorldClockState>>,
    mut text_content: ResMut<Text3dContent>,
) {
    use jiff::Timestamp;

    let now = Timestamp::now();

    let countdown_str: Option<String> = world_clock.and_then(|wc| {
        wc.alarms
            .iter()
            .filter(|a| a.target_ts > now)
            .min_by_key(|a| a.target_ts.as_second())
            .map(|a| {
                let secs = (a.target_ts.as_second() - now.as_second()).max(0) as u64;
                let cd =
                    humantime::format_duration(std::time::Duration::from_secs(secs)).to_string();
                match &a.label {
                    Some(lbl) => format!("{} in {}", lbl, cd),
                    None => cd,
                }
            })
    });

    let fallback = text_content.default_text.clone();
    let new_text = if let Some(cd) = countdown_str {
        cd
    } else {
        match active_post.0 {
            Some(entity) => display_q
                .get(entity)
                .map(|d| d.0.to_string())
                .unwrap_or_else(|_| fallback.clone()),
            None => fallback,
        }
    };

    if text_content.text != new_text {
        text_content.text = new_text;
    }
}

/// Owns the `Text3d` entity lifecycle.
///
/// - No `ShowText3d` present = despawn all `Text3d` entities.
/// - `ShowText3d` present = (re)spaw whenever `Text3dContent`,
///   `Text3dRenderer`, or `Text3dDepth` changes, or no entity
///   yet exists.
#[allow(clippy::too_many_arguments)]
pub fn manage_text3d(
    show_query: Query<(), With<ShowText3d>>,
    content: Res<Text3dContent>,
    renderer: Res<Text3dRenderer>,
    depth: Res<Text3dDepth>,
    existing: Query<Entity, With<Text3d>>,
    mut slug_materials: ResMut<Assets<flan::SlugMaterial>>,
    flan_font: Option<Res<FlanFontId>>,
    mut commands: Commands,
) {
    let should_show = !show_query.is_empty();
    let existing_entities: Vec<Entity> = existing.iter().collect();
    let has_text = !existing_entities.is_empty();

    if !should_show {
        for entity in existing_entities {
            commands.entity(entity).despawn();
        }
        return;
    }

    if content.is_changed() || renderer.is_changed() || depth.is_changed() || !has_text {
        for entity in existing_entities {
            commands.entity(entity).despawn();
        }
        spawn_text3d(
            &mut commands,
            &mut slug_materials,
            flan_font.as_deref(),
            &content.text,
            *renderer,
            depth.0,
        );
    }
}

/// Spawn a `Text3d` entity using the currently selected renderer.
///
/// `depth` is the extrusion depth in world units sourced from `Text3dDepth`. It
/// is only applied for `Text3dRenderer::SlugText3d` and the flat
/// `Text3dRenderer::SlugText` renderer uses `depth: None` for a decal effect.
pub fn spawn_text3d(
    commands: &mut Commands,
    slug_materials: &mut Assets<flan::SlugMaterial>,
    flan_font: Option<&FlanFontId>,
    text: &str,
    renderer: Text3dRenderer,
    depth: f32,
) {
    let transform = Transform::from_xyz(0.0, 0.7, 0.0).with_scale(Vec3::splat(0.5));

    match renderer {
        Text3dRenderer::SlugText => {
            let Some(font) = flan_font else {
                bevy::log::warn!("SlugText: font not ready yet, skipping spawn");
                return;
            };
            let material = slug_materials.add(flan::SlugMaterial::default());
            commands.spawn((
                flan::SlugTextMesh {
                    node: flan::SlugTextNode {
                        text: text.to_string(),
                        color: [0, 0, 0, 255],
                        ..default()
                    },
                    font: flan::SlugTextFont(font.0),
                    material: MeshMaterial3d(material),
                    transform,
                    ..default()
                },
                SlugTextAnchor(transform.translation),
                Text3d,
            ));
        }
        Text3dRenderer::SlugText3d => {
            let Some(font) = flan_font else {
                bevy::log::warn!("SlugText3d: font not ready yet, skipping spawn");
                return;
            };
            let material = slug_materials.add(flan::SlugMaterial::default());
            commands.spawn((
                flan::SlugTextMesh {
                    node: flan::SlugTextNode {
                        text: text.to_string(),
                        color: [0, 0, 0, 255],
                        depth: Some(depth),
                        ..default()
                    },
                    font: flan::SlugTextFont(font.0),
                    material: MeshMaterial3d(material),
                    transform,
                    ..default()
                },
                SlugTextAnchor(transform.translation),
                Text3d,
            ));
        }
    }
}

/// Recenter `SlugText` entities once the mesh `Aabb` has been computed.
///
/// Runs in `PostUpdate` after `CalculateBounds` so the `Aabb` is always correct
/// for the current entity.
#[allow(clippy::type_complexity)]
pub fn center_slug_text(
    mut query: Query<(&mut Transform, &Aabb, &SlugTextAnchor), (With<Text3d>, Changed<Aabb>)>,
) {
    for (mut transform, aabb, anchor) in query.iter_mut() {
        transform.translation.x = anchor.0.x - transform.scale.x * aabb.center.x;
        transform.translation.y = anchor.0.y - transform.scale.y * aabb.center.y;
    }
}

pub struct Text3dPlugin;

impl Plugin for Text3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Text3dContent>()
            .init_resource::<Text3dDefaultPending>()
            .init_resource::<Text3dRenderer>()
            .init_resource::<Text3dDepth>()
            .init_resource::<Text3dDepthPending>()
            .add_systems(Startup, (setup_3d_text, setup_flan_font))
            .add_systems(Update, mark_pending_commits)
            .add_systems(
                Update,
                commit_pending_default
                    .run_if(any_with_component::<Text3dCommitPending>.and(
                        bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(
                            400,
                        )),
                    ))
                    .before(manage_text3d),
            )
            .add_systems(
                Update,
                commit_pending_depth
                    .run_if(any_with_component::<Text3dDepthCommitPending>.and(
                        bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(
                            300,
                        )),
                    ))
                    .before(manage_text3d),
            )
            .add_systems(Update, manage_text3d)
            .add_systems(
                Update,
                sync_text3d_to_active_reverie.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
            )
            .add_systems(
                PostUpdate,
                center_slug_text.after(VisibilitySystems::CalculateBounds),
            );
    }
}
