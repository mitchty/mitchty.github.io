//! 3-D text plugin, this needs to die with slugtext in the near future.
//!
//! Owns the two-phase commit system that debounces text updates to avoid
//! churning mesh entities, the renderer selector, the slug-text centering
//! system, and the sync from the active reverie world-clock alarm systems.

use crate::assets::asset_path;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy_fontmesh::prelude::*;

use crate::plugins::hue::HueAnimationEnabled;
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName};

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

/// Which crate is used to render the 3-D text label.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text3dRenderer {
    FontMesh,
    #[default]
    SlugText,
}

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

/// Startup: spawn the `ShowText3d` marker so text is visible by default.
pub fn setup_3d_text(mut commands: Commands) {
    commands.spawn(ShowText3d);
}

/// Watches `Text3dDefaultPending` for changes and spawns a
/// `Text3dCommitPending` marker if one is not already present.
pub fn mark_pending_commit(
    pending: Res<Text3dDefaultPending>,
    existing: Query<(), With<Text3dCommitPending>>,
    mut commands: Commands,
) {
    if pending.is_changed() && existing.is_empty() {
        commands.spawn(Text3dCommitPending);
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
/// - `ShowText3d` present = spawn whenever `Text3dContent` changes or no
///   entity exists.
pub fn manage_text3d(
    show_query: Query<(), With<ShowText3d>>,
    content: Res<Text3dContent>,
    renderer: Res<Text3dRenderer>,
    existing: Query<Entity, With<Text3d>>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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

    if content.is_changed() || renderer.is_changed() || !has_text {
        for entity in existing_entities {
            commands.entity(entity).despawn();
        }
        spawn_text3d(
            &mut commands,
            &asset_server,
            &mut materials,
            &content.text,
            *renderer,
        );
    }
}

/// Spawn a `Text3d` entity using the currently selected renderer.
pub fn spawn_text3d(
    commands: &mut Commands,
    asset_server: &AssetServer,
    materials: &mut Assets<StandardMaterial>,
    text: &str,
    renderer: Text3dRenderer,
) {
    let font_path = asset_path("fonts/NotoSansJP-Regular.ttf");
    let transform = Transform::from_xyz(0.0, 0.7, 0.0).with_scale(Vec3::splat(0.5));

    match renderer {
        Text3dRenderer::FontMesh => {
            let text_material = materials.add(StandardMaterial {
                base_color: Color::from(Hsla::hsl(180.0, 1.0, 0.5)),
                metallic: 0.5,
                perceptual_roughness: 0.3,
                ..default()
            });
            commands.spawn((
                TextMeshBundle {
                    text_mesh: bevy_fontmesh::prelude::TextMesh {
                        text: text.to_string(),
                        font: asset_server.load(font_path),
                        style: TextMeshStyle {
                            depth: 0.1,
                            subdivision: 20,
                            anchor: TextAnchor::Center,
                            justify: JustifyText::Center,
                        },
                    },
                    material: MeshMaterial3d(text_material),
                    transform,
                    ..default()
                },
                HueAnimationEnabled,
                Text3d,
            ));
        }
        Text3dRenderer::SlugText => {
            commands.spawn((
                bevy_slugtext::prelude::TextMesh {
                    text: text.to_string(),
                    font: asset_server.load(font_path),
                    color: Color::BLACK,
                    bg_color: Color::NONE,
                    size: 1.0,
                },
                transform,
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
            .add_systems(Startup, setup_3d_text)
            .add_systems(Update, mark_pending_commit)
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
