//! Camera plugin: spawning, free-look orbit, drag state, and zoom for now, not
//! sure if this is the right abstraction level but this repo is for learning so
//! why not minot?

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;

use crate::assets::asset_path;
use crate::plugins::fullscreen::{CameraConfig, CameraOrbit};
use mitchty::RenderLayers;
use transform_gizmo_bevy::prelude::GizmoCamera;

use flan::post_process::{EffectsEnabled, PostProcessSettings};

/// Marker component for the main camera.
#[derive(Component)]
pub struct MainCamera;

/// Marker component to indicate camera should be auto-rotating.
#[allow(dead_code)]
#[derive(Component)]
pub struct CameraRotationEnabled;

/// Resource tracking mouse and touch drag state for the free-look camera.
#[derive(Resource, Default)]
pub struct DragState {
    /// Are we actively dragging?
    pub is_dragging: bool,
    /// Starting position when mouse button was pressed or touch started.
    pub drag_start: Option<Vec2>,
    /// Current mouse/touch position.
    pub current_pos: Vec2,
    /// Total distance dragged from start.
    pub drag_distance: f32,
    /// Active primary touch ID.
    pub active_touch_id: Option<u64>,
    /// Previous position for delta calculation.
    pub previous_pos: Option<Vec2>,
    /// Second finger touch ID for two-finger zoom.
    pub secondary_touch_id: Option<u64>,
    /// Current position of the second finger.
    pub secondary_touch_pos: Option<Vec2>,
    /// Distance between the two fingers on the previous frame.
    pub previous_pinch_distance: Option<f32>,
}

/// Free-look camera component carrying current yaw/pitch/sensitivity.
#[derive(Component, Clone, Copy)]
pub struct FreeLookCamera {
    /// Yaw in radians.
    pub yaw: f32,
    /// Pitch in radians.
    pub pitch: f32,
    /// Mouse/touch sensitivity multiplier.
    pub sensitivity: f32,
}

/// Marker component indicating free-look is currently active.
///
/// Stores the last interaction time so that auto-rotate can resume after
/// `INACTIVITY_THRESHOLD` seconds of no input.
#[derive(Component)]
pub struct FreeLookActive(pub f64);

/// startup system to spawn the single scene camera, for now there is only one
/// highlander style.
pub fn spawn_camera(
    mut commands: Commands,
    config: Res<CameraConfig>,
    asset_server: Res<AssetServer>,
    // Option because PostProcessPlugin may be disabled via --without-plugins.
    // When None the camera is spawned without PostProcessSettings and the
    // post-process render node is simply absent.
    effects_enabled: Option<Res<EffectsEnabled>>,
) {
    let diffuse_path = asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    let specular_path = asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");

    let mut camera = commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            ..default()
        },
        config.transform,
        EnvironmentMapLight {
            diffuse_map: asset_server.load(&diffuse_path),
            specular_map: asset_server.load(&specular_path),
            intensity: 2_000.0,
            ..default()
        },
        config.free_look,
        config.orbit,
        MainCamera,
        GizmoCamera,
        RenderLayers::layer(0),
    ));

    if let Some(enabled) = effects_enabled {
        camera.insert(PostProcessSettings {
            intensity: if enabled.0 { 1.0 } else { 0.0 },
            ..Default::default()
        });
    }
}

// TODO: I need to think through how I want to handle all input slightly better
// than the current "dunno lets copy/paste code from the bevy examples and hope
// for the best" approach
/// Track mouse and touch drag state for distinguishing clicks from drags.
pub fn track_input_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<CursorMoved>,
    mut touch_events: MessageReader<TouchInput>,
    mut drag_state: ResMut<DragState>,
    interaction_query: Query<&Interaction>,
    gizmo_target_query: Query<&transform_gizmo_bevy::prelude::GizmoTarget>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    let ui_is_interacted = interaction_query
        .iter()
        .any(|interaction| *interaction != Interaction::None);

    #[cfg(feature = "egui")]
    let egui_is_using_input = egui_wants_input.wants_pointer;
    #[cfg(not(feature = "egui"))]
    let egui_is_using_input = false;

    let gizmo_is_active = gizmo_target_query
        .iter()
        .any(|t| t.latest_result().is_some());

    if ui_is_interacted || egui_is_using_input || gizmo_is_active {
        drag_state.is_dragging = false;
        drag_state.drag_start = None;
        drag_state.active_touch_id = None;
        drag_state.previous_pos = None;
        return;
    }

    if mouse.just_pressed(MouseButton::Left) {
        drag_state.is_dragging = true;
        drag_state.drag_start = Some(drag_state.current_pos);
        drag_state.previous_pos = Some(drag_state.current_pos);
        drag_state.drag_distance = 0.0;
        drag_state.active_touch_id = None;
    }

    if mouse.just_released(MouseButton::Left) {
        drag_state.is_dragging = false;
        drag_state.drag_start = None;
        drag_state.previous_pos = None;
    }

    for event in motion.read() {
        drag_state.current_pos = event.position;
        if drag_state.is_dragging
            && let Some(start) = drag_state.drag_start
        {
            drag_state.drag_distance = start.distance(drag_state.current_pos);
        }
    }

    for event in touch_events.read() {
        match event.phase {
            bevy::input::touch::TouchPhase::Started => {
                if drag_state.active_touch_id.is_none() {
                    drag_state.is_dragging = true;
                    drag_state.drag_start = Some(event.position);
                    drag_state.current_pos = event.position;
                    drag_state.previous_pos = Some(event.position);
                    drag_state.drag_distance = 0.0;
                    drag_state.active_touch_id = Some(event.id);
                } else if drag_state.secondary_touch_id.is_none()
                    && Some(event.id) != drag_state.active_touch_id
                {
                    drag_state.secondary_touch_id = Some(event.id);
                    drag_state.secondary_touch_pos = Some(event.position);
                    drag_state.previous_pinch_distance =
                        Some(drag_state.current_pos.distance(event.position));
                }
            }
            bevy::input::touch::TouchPhase::Moved => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.current_pos = event.position;
                    if let Some(start) = drag_state.drag_start {
                        drag_state.drag_distance = start.distance(drag_state.current_pos);
                    }
                } else if Some(event.id) == drag_state.secondary_touch_id {
                    drag_state.secondary_touch_pos = Some(event.position);
                }
            }
            bevy::input::touch::TouchPhase::Ended | bevy::input::touch::TouchPhase::Canceled => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.is_dragging = false;
                    drag_state.drag_start = None;
                    drag_state.previous_pos = None;
                    drag_state.active_touch_id = None;
                    if let Some(sec_id) = drag_state.secondary_touch_id {
                        drag_state.active_touch_id = Some(sec_id);
                        drag_state.current_pos = drag_state.secondary_touch_pos.unwrap_or_default();
                        drag_state.previous_pos = drag_state.secondary_touch_pos;
                        drag_state.drag_start = drag_state.secondary_touch_pos;
                        drag_state.secondary_touch_id = None;
                        drag_state.secondary_touch_pos = None;
                        drag_state.previous_pinch_distance = None;
                        drag_state.is_dragging = true;
                    }
                } else if Some(event.id) == drag_state.secondary_touch_id {
                    drag_state.secondary_touch_id = None;
                    drag_state.secondary_touch_pos = None;
                    drag_state.previous_pinch_distance = None;
                }
            }
        }
    }
}

/// Free-look camera just rotates the camera around the orbit center based on
/// mouse or single-finger touch drag. Note the touch pad input is weeeeird and
/// likely wrong. I rarely test it future mitch problem to fix more generally.
pub fn free_look_camera(
    mouse: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<CursorMoved>,
    mut touch_events: MessageReader<TouchInput>,
    mut drag_state: ResMut<DragState>,
    time: Res<Time>,
    mut camera_query: Query<
        (Entity, &mut Transform, &mut FreeLookCamera, &CameraOrbit),
        With<MainCamera>,
    >,
    mut commands: Commands,
) {
    if !drag_state.is_dragging || drag_state.drag_distance < 5.0 {
        return;
    }

    if drag_state.secondary_touch_id.is_some() {
        return;
    }

    let Ok((entity, mut transform, mut free_look, orbit)) = camera_query.single_mut() else {
        return;
    };

    if mouse.pressed(MouseButton::Left) {
        for event in mouse_motion.read() {
            if let Some(delta) = event.delta {
                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;
                free_look.pitch = free_look.pitch.clamp(-1.5, 1.5);

                let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
                let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
                let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();

                transform.translation = Vec3::new(x, y, z);
                *transform = transform.looking_at(orbit.center, Vec3::Y);

                drag_state.previous_pos = Some(event.position);
            }
        }
    }

    for event in touch_events.read() {
        if event.phase == bevy::input::touch::TouchPhase::Moved
            && Some(event.id) == drag_state.active_touch_id
        {
            if let Some(prev_pos) = drag_state.previous_pos {
                let delta = event.position - prev_pos;

                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;
                free_look.pitch = free_look.pitch.clamp(-1.5, 1.5);

                let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
                let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
                let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();

                transform.translation = Vec3::new(x, y, z);
                *transform = transform.looking_at(orbit.center, Vec3::Y);
            }
            drag_state.previous_pos = Some(event.position);
        }
    }
}

/// Remove `FreeLookActive` after 2 seconds of inactivity so auto-rotation
/// can resume.
// TODO: This is largely dead code now but I might bring it back
pub fn cleanup_free_look_after_inactivity(
    time: Res<Time>,
    query: Query<(Entity, &FreeLookActive)>,
    mut commands: Commands,
) {
    const INACTIVITY_THRESHOLD: f64 = 2.0;
    let current_time = time.elapsed_secs_f64();
    for (entity, free_look) in query.iter() {
        if current_time - free_look.0 > INACTIVITY_THRESHOLD {
            commands.entity(entity).remove::<FreeLookActive>();
        }
    }
}

/// Zoom the camera via mouse wheel or two-finger pinch.
///
/// In **Perspective** mode the orbital radius is adjusted.
/// In **Orthographic** mode the projection scale is adjusted instead.
pub fn zoom_camera(
    mut wheel: MessageReader<MouseWheel>,
    mut drag_state: ResMut<DragState>,
    mut camera_query: Query<
        (
            &mut Transform,
            &mut CameraOrbit,
            &FreeLookCamera,
            &mut Projection,
        ),
        With<MainCamera>,
    >,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_pointer {
        return;
    }

    let Ok((mut transform, mut orbit, free_look, mut projection)) = camera_query.single_mut()
    else {
        return;
    };

    let mut zoom_delta: f32 = 0.0;

    for event in wheel.read() {
        let scroll = match event.unit {
            MouseScrollUnit::Line => event.y * 0.5,
            MouseScrollUnit::Pixel => event.y * 0.005,
        };
        zoom_delta += scroll;
    }

    if let (Some(sec_pos), Some(prev_dist)) = (
        drag_state.secondary_touch_pos,
        drag_state.previous_pinch_distance,
    ) {
        let current_dist = drag_state.current_pos.distance(sec_pos);
        zoom_delta += (current_dist - prev_dist) * 0.01;
        drag_state.previous_pinch_distance = Some(current_dist);
    }

    if zoom_delta == 0.0 {
        return;
    }

    match *projection {
        Projection::Perspective(_) => {
            orbit.radius = (orbit.radius - zoom_delta).max(0.0);
            let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
            let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
            let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();
            transform.translation = Vec3::new(x, y, z);
            *transform = transform.looking_at(orbit.center, Vec3::Y);
        }
        Projection::Orthographic(ref mut ortho) => {
            ortho.scale = (ortho.scale - zoom_delta).max(0.0);
        }
        _ => {}
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<mitchty::CameraMode>()
            .init_resource::<DragState>()
            .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    track_input_drag,
                    free_look_camera,
                    cleanup_free_look_after_inactivity,
                    zoom_camera,
                ),
            );
    }
}
