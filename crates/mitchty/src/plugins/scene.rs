//! GLTF/GLB scene lifecycle plugin.
//!
//! Owns all resources and systems related to loading, replacing, transforming,
//! and toggling visibility of the main 3-D scene. Also bridges the
//! `transform-gizmo-bevy` gizmo interaction, post-process disabling during
//! gizmo use, and device-orientation events from the Losant SSE stream data
// TODO: This does too much but this all being alongside main() was worse but
// the sse streaming stuffs a bit of a smell here if I'm honest.

use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneSpawner};
use transform_gizmo_bevy::{GizmoDragStarted, GizmoDragging, prelude::*};

use crate::assets::asset_path;

use crate::plugins::camera::DragState;
use crate::post_process::EffectsEnabled;

/// Resource holding the optional background color override.
///
/// `None`    =  follow the active dark/light theme.
/// `Some(c)` = use this color directly.
#[derive(Resource)]
pub struct ColorState {
    pub color: Option<Srgba>,
}

/// Tracks the loaded GLB/GLTF scene.
///
/// `custom_scene` holds whatever string the user supplied aka URI or file path.
/// It is passed directly to `AssetServer::load` which routes it to the correct
/// `AssetSource` automatically. `None` means use the embedded compile-time GLB.
#[derive(Resource, Default)]
pub struct SceneConfig {
    pub custom_scene: Option<String>,
}

/// Full `Transform` applied to the loaded GLTF/GLB scene root entity.
///
/// Defaults to uniform scale of `0.3`. Mutating this resource causes
/// `apply_scene_transform` to write the new transform on the next frame/ecs
/// tick.
#[derive(Resource, Clone)]
pub struct SceneTransformConfig {
    pub transform: Transform,
}

impl Default for SceneTransformConfig {
    fn default() -> Self {
        Self {
            transform: Transform::from_scale(Vec3::splat(0.3)),
        }
    }
}

/// State for the "Load Scene URL" popup window.
#[derive(Resource, Default)]
pub struct SceneUrlState {
    /// Whether the popup is currently open.
    pub open: bool,
    /// Text buffer bound to the URL `TextEdit`.
    pub buf: String,
    /// Confirmed URL -> triggers `SceneConfig` update.
    pub confirmed_url: Option<String>,
}

/// Marker for the current GLTF/GLB scene root entity.
#[derive(Component)]
pub struct LoadedScene;

/// Marker controlling whether the loaded GLTF/GLB model is rendered.
#[derive(Component, Default)]
pub struct ShowSceneModel;

/// Startup: spawn the default embedded GLTF scene.
pub fn setup(
    asset_server: Res<AssetServer>,
    scene_transform: Res<SceneTransformConfig>,
    mut commands: Commands,
) {
    let glb_path = asset_path("mitchty.glb");
    let scene_handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(glb_path));

    commands
        .spawn((
            SceneRoot(scene_handle),
            scene_transform.transform,
            LoadedScene,
        ))
        .observe(on_scene_ready);

    commands.spawn(ShowSceneModel);
}

/// Watches `SceneConfig` for changes and swaps out the active scene.
///
/// Also resets `SceneTransformConfig` to defaults and opens the Scene Config
/// window on every new load.
pub fn replace_scene(
    config: Res<SceneConfig>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
    scene_query: Query<Entity, With<LoadedScene>>,
    mut scene_transform: ResMut<SceneTransformConfig>,
    scene_cfg_window_query: Query<(), With<crate::ui::ShowSceneConfig>>,
) {
    if !config.is_changed() || config.is_added() {
        return;
    }

    for entity in scene_query.iter() {
        commands.entity(entity).despawn();
    }

    *scene_transform = SceneTransformConfig::default();

    if scene_cfg_window_query.is_empty() {
        commands.spawn(crate::ui::ShowSceneConfig);
    }

    let path = match &config.custom_scene {
        Some(s) => s.clone(),
        None => asset_path("mitchty.glb"),
    };

    let scene_handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(path));
    commands
        .spawn((
            SceneRoot(scene_handle),
            scene_transform.transform,
            LoadedScene,
        ))
        .observe(on_scene_ready);
}

/// Adds or removes `GizmoTarget` from the `LoadedScene` entity based on whether
/// the Scene Config panel is currently visible.
pub fn sync_gizmo_target(
    scene_config_query: Query<(), With<crate::ui::ShowSceneConfig>>,
    scene_query: Query<(Entity, Has<GizmoTarget>), With<LoadedScene>>,
    mut commands: Commands,
) {
    let wants_gizmo = !scene_config_query.is_empty();
    for (entity, has_target) in scene_query.iter() {
        if wants_gizmo && !has_target {
            commands.entity(entity).insert(GizmoTarget::default());
        } else if !wants_gizmo && has_target {
            commands.entity(entity).remove::<GizmoTarget>();
        }
    }
}

/// Disables post-process effects while the Scene Config panel is open and
/// restores the previous state when it closes.
// TODO: and I need to fix this bug but I need to have a marker component to be
// sure that effect processing via pressing e can't work when the scene config
// panel is open.
pub fn manage_post_process_for_gizmo(
    added: Query<(), Added<crate::ui::ShowSceneConfig>>,
    mut removed: RemovedComponents<crate::ui::ShowSceneConfig>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    mut was_enabled: Local<bool>,
) {
    if !added.is_empty() {
        *was_enabled = effects_enabled.0;
        effects_enabled.0 = false;
    }
    if removed.read().next().is_some() {
        effects_enabled.0 = *was_enabled;
    }
}

/// Keeps `SceneTransformConfig` in sync with what `TransformGizmoPlugin`
/// writes directly to the `LoadedScene` entity's `Transform`.
pub fn sync_scene_config_from_gizmo(
    scene_query: Query<(&Transform, &GizmoTarget), With<LoadedScene>>,
    mut scene_transform: ResMut<SceneTransformConfig>,
) {
    for (transform, gizmo_target) in scene_query.iter() {
        if !gizmo_target.is_active() {
            continue;
        }
        match gizmo_target.latest_result() {
            Some(GizmoResult::Scale { .. }) => {
                scene_transform.bypass_change_detection().transform.scale = transform.scale;
                scene_transform.bypass_change_detection().transform.rotation = transform.rotation;
            }
            Some(_) => {
                scene_transform.bypass_change_detection().transform = *transform;
            }
            None => {}
        }
    }
}

/// Ensures a gizmo scale operation never displaces the scene root in world
/// space by resetting translation to the authoritative value in `SceneTransformConfig`.
pub fn preserve_origin_on_scale(
    mut scene_query: Query<(&mut Transform, &GizmoTarget), With<LoadedScene>>,
    scene_config: Res<SceneTransformConfig>,
) {
    for (mut transform, gizmo_target) in scene_query.iter_mut() {
        if let Some(GizmoResult::Scale { .. }) = gizmo_target.latest_result() {
            let pinned = scene_config.transform.translation;
            if transform.translation != pinned {
                transform.translation = pinned;
            }
        }
    }
}

/// Bridges single-finger touch input into the `GizmoDragStarted` / `GizmoDragging`
/// events that `transform-gizmo-bevy` expects as touch support doesn't work sadly.
pub fn touch_interact_gizmo(
    mut touch_events: MessageReader<TouchInput>,
    mut drag_started: MessageWriter<GizmoDragStarted>,
    mut dragging: MessageWriter<GizmoDragging>,
    gizmo_target_query: Query<(), With<GizmoTarget>>,
    drag_state: Res<DragState>,
) {
    if gizmo_target_query.is_empty() {
        return;
    }

    for event in touch_events.read() {
        let is_primary =
            drag_state.active_touch_id.is_none() || drag_state.active_touch_id == Some(event.id);

        if !is_primary {
            continue;
        }

        match event.phase {
            bevy::input::touch::TouchPhase::Started => {
                drag_started.write_default();
                dragging.write_default();
            }
            bevy::input::touch::TouchPhase::Moved => {
                dragging.write_default();
            }
            _ => {}
        }
    }
}

/// Watches `SceneTransformConfig` for changes and writes the stored `Transform`
/// to the live `LoadedScene` root entity.
///
/// Scale is soft-clamped to `0.001` so a degenerate zero/negative scale can't
/// confuse Bevy's renderer.
pub fn apply_scene_transform(
    scene_transform: Res<SceneTransformConfig>,
    mut scene_query: Query<&mut Transform, With<LoadedScene>>,
) {
    if !scene_transform.is_changed() {
        return;
    }
    let mut t = scene_transform.transform;
    t.scale = t.scale.max(Vec3::splat(0.001));
    for mut transform in scene_query.iter_mut() {
        *transform = t;
    }
}

/// Mirrors the `ShowSceneModel` marker onto each `LoadedScene` entity's
/// `Visibility`.
pub fn apply_scene_model_visibility(
    show_query: Query<(), With<ShowSceneModel>>,
    mut scene_query: Query<&mut Visibility, With<LoadedScene>>,
) {
    let target = if show_query.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    for mut vis in scene_query.iter_mut() {
        if *vis != target {
            *vis = target;
        }
    }
}

/// Observer attached to each `SceneRoot` entity via `.observe()`.
///
/// Fires on `SceneInstanceReady` and strips any embedded `Camera3d`, `Camera`,
/// `PointLight`, `DirectionalLight`, and `SpotLight` entities that Blender or
/// anyone else might have baked into the GLTF file. Future me should make this
/// less derp.
pub fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    scene_spawner: Res<SceneSpawner>,
    mut commands: Commands,
    cameras: Query<(), With<Camera3d>>,
    point_lights: Query<(), With<PointLight>>,
    dir_lights: Query<(), With<DirectionalLight>>,
    spot_lights: Query<(), With<SpotLight>>,
) {
    for entity in scene_spawner.iter_instance_entities(trigger.event().instance_id) {
        if cameras.contains(entity) {
            bevy::log::debug!(
                "on_scene_ready: removing embedded Camera3d from {:?}",
                entity
            );
            commands
                .entity(entity)
                .remove::<Camera3d>()
                .remove::<Camera>();
        }
        if point_lights.contains(entity) {
            bevy::log::debug!(
                "on_scene_ready: removing embedded PointLight from {:?}",
                entity
            );
            commands.entity(entity).remove::<PointLight>();
        }
        if dir_lights.contains(entity) {
            bevy::log::debug!(
                "on_scene_ready: removing embedded DirectionalLight from {:?}",
                entity
            );
            commands.entity(entity).remove::<DirectionalLight>();
        }
        if spot_lights.contains(entity) {
            bevy::log::debug!(
                "on_scene_ready: removing embedded SpotLight from {:?}",
                entity
            );
            commands.entity(entity).remove::<SpotLight>();
        }
    }
}

/// Syncs background clear color to the `ColorState` resource.
///
/// When `color_state.color` is `None` the color follows the active dark/light
/// theme and `Some(c)` overrides it directly. I really need to figure out a
/// themeing strategy.
pub fn sync_color_state_to_clear_color(
    color_state: Res<ColorState>,
    ui_config: Res<crate::ui::UiConfig>,
    mut clear_color: ResMut<ClearColor>,
) {
    if color_state.is_changed() || ui_config.is_changed() {
        let resolved = match color_state.color {
            Some(c) => c,
            None => {
                let is_dark = match ui_config.theme {
                    crate::ui::ThemeChoice::Dark => true,
                    crate::ui::ThemeChoice::Light => false,
                    crate::ui::ThemeChoice::Auto => {
                        match dark_light::detect().unwrap_or(dark_light::Mode::Unspecified) {
                            dark_light::Mode::Dark => true,
                            dark_light::Mode::Light | dark_light::Mode::Unspecified => false,
                        }
                    }
                };
                if is_dark {
                    Srgba::new(0.0, 0.0, 0.0, 1.0)
                } else {
                    Srgba::new(1.0, 1.0, 1.0, 1.0)
                }
            }
        };
        clear_color.0 = resolved.into();
    }
}

/// Applies live device orientation received over the Losant SSE state stream.
///
/// Converts `roll` and `pitch` in degrees [-180, 180] from a physical device IMU into
/// a scene rotation, leaving the camera undisturbed.
pub fn apply_device_state(
    mut device_events: bevy::ecs::message::MessageReader<crate::ui::losant::DeviceStateEvent>,
    mut scene_transform: ResMut<SceneTransformConfig>,
) {
    for event in device_events.read() {
        let data = &event.0;
        let rotation = Quat::from_euler(
            EulerRot::YXZ,
            data.roll.to_radians(),
            data.pitch.to_radians(),
            0.0,
        );
        scene_transform.transform.rotation = rotation;
    }
}

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SceneConfig>()
            .init_resource::<SceneUrlState>()
            .init_resource::<SceneTransformConfig>()
            .insert_resource(ColorState { color: None })
            .add_systems(Startup, setup)
            .add_systems(Update, replace_scene)
            .add_systems(Update, apply_scene_transform)
            .add_systems(Update, apply_scene_model_visibility)
            .add_systems(Update, sync_gizmo_target)
            .add_systems(Update, touch_interact_gizmo)
            .add_systems(Update, manage_post_process_for_gizmo)
            .add_systems(
                Update,
                (sync_scene_config_from_gizmo, preserve_origin_on_scale).chain(),
            )
            .add_systems(Update, sync_color_state_to_clear_color)
            .add_systems(Update, apply_device_state);
    }
}
