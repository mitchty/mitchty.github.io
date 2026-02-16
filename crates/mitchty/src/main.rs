mod assets;
mod fullscreen_effect;
mod post_process;
mod ui;

#[cfg(feature = "feathers")]
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::prelude::*;

/// Resource to hold the current background color state
#[derive(Resource)]
pub struct ColorState {
    pub color: Srgba,
}

// use bevy_old_tv_shader::prelude::*;
use rand::Rng;

use assets::{AssetConfigPlugin, asset_path};
use bevy_fontmesh::prelude::*;
use fullscreen_effect::{
    CameraConfig, CameraOrbit, manage_effect_settings, next_effect, previous_effect, spawn_camera,
    toggle_fullscreen_effect, update_effect_time,
};
use post_process::PostProcessPlugin;
use ui::SettingsUiPlugin;

/// Absolute rotation speed
const SPEED: f32 = 2.25;
/// Minimum rotation speed in radians per second
const MIN_SPEED: f32 = -SPEED;
/// Maximum rotation speed in radians per second
const MAX_SPEED: f32 = SPEED;
/// Golden angle for rotation calculations
const GOLDEN_ANGLE: f32 = 137.507_77;

/// Marker component for entities that should rotate
#[derive(Component)]
struct Rotator {
    /// Base rotation speed in radians per second for each axis (x, y, z)
    base_speed: Vec3,
}

/// Marker component to indicate cube rotation is enabled (on cube entities)
#[derive(Component)]
pub struct CubeRotationEnabled;

/// Marker component separate from cube entities to control rotation
#[derive(Component, Default)]
pub struct CubeRotation;

/// Marker component to indicate hue animation is enabled (on cube entities)
#[derive(Component)]
pub struct HueAnimationEnabled;

/// Marker component separate from cube entities to control hue animation
#[derive(Component, Default)]
pub struct HueAnimation;

/// Marker component for the FPS text entity
#[derive(Component)]
struct FpsText;

/// Marker component to indicate FPS should be displayed and updated
#[derive(Component, Default)]
pub struct FpsDisplay;

/// Marker component to indicate camera should be rotating
#[derive(Component)]
pub struct CameraRotationEnabled;

/// Marker component for the main camera to enable TV effect toggling
#[derive(Component)]
pub struct MainCamera;

/// Resource to track mouse and touch drag state for free-look camera
#[derive(Resource, Default)]
pub struct DragState {
    /// Are we actively dragging or not?
    pub is_dragging: bool,
    /// Starting position when mouse button was pressed or touch started
    pub drag_start: Option<Vec2>,
    /// Current mouse/touch position
    pub current_pos: Vec2,
    /// Total distance dragged from start
    pub drag_distance: f32,
    /// Active touch ID iff dragging
    pub active_touch_id: Option<u64>,
    /// Previous position for calculating deltas between drags
    pub previous_pos: Option<Vec2>,
}

/// Camera free-look component
#[derive(Component, Clone, Copy)]
pub struct FreeLookCamera {
    /// Yaw in radians
    pub yaw: f32,
    /// Pitch in radians
    pub pitch: f32,
    /// Sensitivity?
    pub sensitivity: f32,
}

/// Marker component indicating free-look is currently active (blocks automatic rotation)
/// Stores the last interaction time so that auto rotate can resume if on
#[derive(Component)]
struct FreeLookActive(f64);

/// TODO: This files getting obscenely too long time to start splitting stuff up.
fn main() {
    // Set up better panic messages for WASM for when this stuff seems to not
    // work or I manage to use a library that won't run on it without paying
    // attention... again.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    let mut app = App::new();

    app.add_plugins(assets::create_default_plugins())
        .add_plugins(AssetConfigPlugin)
        .add_plugins(FontMeshPlugin)
        .add_plugins(PostProcessPlugin)
        .insert_resource(ClearColor(Color::srgb(0.5, 0.5, 0.5)))
        .insert_resource(ColorState {
            color: Srgba::gray(0.5),
        })
        .init_resource::<CameraConfig>();

    // Conditionally add UI-specific plugins
    #[cfg(feature = "egui")]
    {
        app.add_plugins(bevy_egui::EguiPlugin::default());
    }

    #[cfg(feature = "feathers")]
    {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()));
    }

    app.add_plugins(SettingsUiPlugin)
        .init_resource::<DragState>()
        .add_systems(Startup, (setup, setup_fps_ui, setup_3d_text, spawn_camera))
        .add_systems(
            Update,
            (
                sync_color_state_to_clear_color,
                track_input_drag,
                free_look_camera,
                cleanup_free_look_after_inactivity,
                animate_materials.run_if(any_with_component::<HueAnimationEnabled>),
                rotate_entities.run_if(any_with_component::<CubeRotationEnabled>),
                toggle_fps_display,
                toggle_cube_rotation,
                toggle_hue_animation,
                toggle_fullscreen_effect,
                next_effect,
                previous_effect,
                apply_cube_rotation,
                apply_hue_animation,
                manage_effect_settings,
                update_effect_time,
                update_fps_display.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs_f32(0.5),
                )),
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // let diffuse_path = asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    // let specular_path = asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");

    let cube = meshes.add(Cuboid::new(0.5, 0.5, 0.5));

    let mut hsla = Hsla::hsl(0.0, 1.0, 0.5);
    let mut rng = rand::rng();

    for x in -1..2 {
        for z in -1..2 {
            let base_speed = Vec3::new(
                rng.random_range(MIN_SPEED..=MAX_SPEED),
                rng.random_range(MIN_SPEED..=MAX_SPEED),
                rng.random_range(MIN_SPEED..=MAX_SPEED),
            );

            commands.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(materials.add(Color::from(hsla))),
                Transform::from_translation(Vec3::new(x as f32, 0.0, z as f32)),
                Rotator { base_speed },
                CubeRotationEnabled,
                HueAnimationEnabled,
            ));
            hsla = hsla.rotate_hue(GOLDEN_ANGLE);
        }
    }
}

/// Cube material animation system
fn animate_materials(
    material_handles: Query<&MeshMaterial3d<StandardMaterial>, With<HueAnimationEnabled>>,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for material_handle in material_handles.iter() {
        if let Some(material) = materials.get_mut(material_handle)
            && let Color::Hsla(ref mut hsla) = material.base_color
        {
            *hsla = hsla.rotate_hue(time.delta_secs() * 100.0);
        }
    }
}

/// Cube random rotation system
fn rotate_entities(
    mut query: Query<(&mut Transform, &mut Rotator), With<CubeRotationEnabled>>,
    time: Res<Time>,
) {
    let mut rng = rand::rng();
    let delta = time.delta_secs();

    for (mut transform, mut rotator) in &mut query {
        let change_x = rng.random_range(-0.1..=0.1);
        let change_y = rng.random_range(-0.1..=0.1);
        let change_z = rng.random_range(-0.1..=0.1);

        rotator.base_speed.x += change_x;
        rotator.base_speed.y += change_y;
        rotator.base_speed.z += change_z;

        if rotator.base_speed.x > MAX_SPEED {
            rotator.base_speed.x = MAX_SPEED - (rotator.base_speed.x - MAX_SPEED);
        }
        if rotator.base_speed.y > MAX_SPEED {
            rotator.base_speed.y = MAX_SPEED - (rotator.base_speed.y - MAX_SPEED);
        }
        if rotator.base_speed.z > MAX_SPEED {
            rotator.base_speed.z = MAX_SPEED - (rotator.base_speed.z - MAX_SPEED);
        }

        if rotator.base_speed.x < MIN_SPEED {
            rotator.base_speed.x = MIN_SPEED + (MIN_SPEED - rotator.base_speed.x);
        }
        if rotator.base_speed.y < MIN_SPEED {
            rotator.base_speed.y = MIN_SPEED + (MIN_SPEED - rotator.base_speed.y);
        }
        if rotator.base_speed.z < MIN_SPEED {
            rotator.base_speed.z = MIN_SPEED + (MIN_SPEED - rotator.base_speed.z);
        }

        transform.rotate_x(rotator.base_speed.x * delta);
        transform.rotate_y(rotator.base_speed.y * delta);
        transform.rotate_z(rotator.base_speed.z * delta);
    }
}

/// System to spawn the fps text entity in the upper right of screen
fn setup_fps_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::srgb(0.0, 1.0, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            right: Val::Px(10.0),
            ..default()
        },
        FpsText,
    ));
}

/// Toggle FpsDisplay marker component to control systems that display the fps text
/// f toggles on/off
fn toggle_fps_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyF) {
        if let Ok(entity) = fps_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(FpsDisplay);
        }
    }
}

/// System to update fps display when toggled
fn update_fps_display(
    time: Res<Time>,
    mut fps_text_query: Query<&mut Text, With<FpsText>>,
    fps_display_query: Query<(), With<FpsDisplay>>,
) {
    if fps_display_query.is_empty() {
        // Cheat, no text when the markers off
        for mut text in fps_text_query.iter_mut() {
            if !text.0.is_empty() {
                text.0.clear();
            }
        }
    } else {
        let fps = 1.0 / time.delta_secs();
        for mut text in fps_text_query.iter_mut() {
            text.0 = format!("{:.1} fps", fps);
        }
    }
}

// TODO: port over the tv effect shader or try making my own with the full
// screen shaders now in 0.18? MORE FUTURE MITCH WORK!
// /// Toggle TV effect on the main camera
// /// t toggles on/off
// fn toggle_tv_effect(
//     keyboard: Res<ButtonInput<KeyCode>>,
//     tv_effect_query: Query<Entity, With<TvEffectEnabled>>,
//     mut commands: Commands,
// ) {
//     if keyboard.just_pressed(KeyCode::KeyT) {
//         if let Ok(entity) = tv_effect_query.single() {
//             commands.entity(entity).despawn();
//         } else {
//             commands.spawn(TvEffectEnabled);
//         }
//     }
// }

// /// Apply or remove TV effect toggle
// fn apply_tv_effect(
//     tv_effect_query: Query<(), With<TvEffectEnabled>>,
//     camera_query: Query<(Entity, Has<OldTvSettings>), With<MainCamera>>,
//     tv_settings: Res<TvSettingsResource>,
//     mut commands: Commands,
// ) {
//     let tv_should_be_enabled = !tv_effect_query.is_empty();

//     for (entity, has_tv_settings) in camera_query.iter() {
//         if tv_should_be_enabled && !has_tv_settings {
//             commands.entity(entity).insert(tv_settings.settings);
//         } else if !tv_should_be_enabled && has_tv_settings {
//             commands.entity(entity).remove::<OldTvSettings>();
//         }
//     }
// }

/// Toggle cube rotation marker
/// c toggles on/off
fn toggle_cube_rotation(
    keyboard: Res<ButtonInput<KeyCode>>,
    rotation_query: Query<Entity, With<CubeRotation>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyC) {
        if let Ok(entity) = rotation_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(CubeRotation);
        }
    }
}

fn apply_cube_rotation(
    rotation_marker: Query<(), With<CubeRotation>>,
    cube_query: Query<(Entity, Has<CubeRotationEnabled>), With<Rotator>>,
    mut commands: Commands,
) {
    let should_rotate = !rotation_marker.is_empty();

    for (entity, has_rotation) in cube_query.iter() {
        if should_rotate && !has_rotation {
            commands.entity(entity).insert(CubeRotationEnabled);
        } else if !should_rotate && has_rotation {
            commands.entity(entity).remove::<CubeRotationEnabled>();
        }
    }
}

/// Toggle hue animation
/// h toggles on/off
fn toggle_hue_animation(
    keyboard: Res<ButtonInput<KeyCode>>,
    hue_query: Query<Entity, With<HueAnimation>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyH) {
        if let Ok(entity) = hue_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(HueAnimation);
        }
    }
}

/// Apply or remove HueAnimationEnabled component toggle
fn apply_hue_animation(
    hue_marker: Query<(), With<HueAnimation>>,
    cube_query: Query<(Entity, Has<HueAnimationEnabled>), With<Rotator>>,
    mut commands: Commands,
) {
    let should_animate = !hue_marker.is_empty();

    for (entity, has_animation) in cube_query.iter() {
        if should_animate && !has_animation {
            commands.entity(entity).insert(HueAnimationEnabled);
        } else if !should_animate && has_animation {
            commands.entity(entity).remove::<HueAnimationEnabled>();
        }
    }
}

#[derive(Component)]
struct Text3d;

/// Setup 3D text "work in progress" above the cubes... future funsies is making this dynamic... future mitch
fn setup_3d_text(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let font_path = asset_path("fonts/FiraMono-Medium.ttf");
    // let font_path = asset_path("fonts/ComicCode-Regular.otf");
    // let font_path = asset_path("fonts/PragmataPro-Regular.ttf");

    let text_material = materials.add(StandardMaterial {
        base_color: Color::from(Hsla::hsl(180.0, 1.0, 0.5)),
        metallic: 0.5,
        perceptual_roughness: 0.3,
        ..default()
    });

    // Spawn the text entities above the cubes for no raisin
    commands.spawn((
        TextMeshBundle {
            text_mesh: TextMesh {
                text: String::from("work in progress"),
                font: asset_server.load(font_path),
                style: TextMeshStyle {
                    depth: 0.1,
                    subdivision: 20,
                    anchor: TextAnchor::Center,
                    justify: JustifyText::Center,
                },
            },
            material: MeshMaterial3d(text_material),
            // TODO: make scale dynamic through the bevy ui settings spiel right
            // now half size methinks
            transform: Transform::from_xyz(0.0, 0.7, 0.0).with_scale(Vec3::splat(0.5)), // Adjust scale here (0.5 = half size)
            ..default()
        },
        HueAnimationEnabled,
        Text3d,
    ));
}

/// Track mouse and touch drag state for distinguishing clicks from drags
fn track_input_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<CursorMoved>,
    mut touch_events: MessageReader<TouchInput>,
    mut drag_state: ResMut<DragState>,
    interaction_query: Query<&Interaction>,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    let ui_is_interacted = interaction_query
        .iter()
        .any(|interaction| *interaction != Interaction::None);

    // Check if egui is using the input
    #[cfg(feature = "egui")]
    let egui_is_using_input = egui_wants_input.wants_pointer;
    #[cfg(not(feature = "egui"))]
    let egui_is_using_input = false;

    if ui_is_interacted || egui_is_using_input {
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
        drag_state.active_touch_id = None; // Clear touch if mouse is used
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

    // Track touch input tooooooo TODO: stop copy/pasting lazy past mitch and do it right
    for event in touch_events.read() {
        match event.phase {
            bevy::input::touch::TouchPhase::Started => {
                // Only track first touch
                if drag_state.active_touch_id.is_none() {
                    drag_state.is_dragging = true;
                    drag_state.drag_start = Some(event.position);
                    drag_state.current_pos = event.position;
                    drag_state.previous_pos = Some(event.position);
                    drag_state.drag_distance = 0.0;
                    drag_state.active_touch_id = Some(event.id);
                }
            }
            bevy::input::touch::TouchPhase::Moved => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.current_pos = event.position;

                    if let Some(start) = drag_state.drag_start {
                        drag_state.drag_distance = start.distance(drag_state.current_pos);
                    }
                }
            }
            bevy::input::touch::TouchPhase::Ended | bevy::input::touch::TouchPhase::Canceled => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.is_dragging = false;
                    drag_state.drag_start = None;
                    drag_state.previous_pos = None;
                    drag_state.active_touch_id = None;
                }
            }
        }
    }
}

/// Free-look camera system - rotates camera based on mouse or touch drag
fn free_look_camera(
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
    // Only apply free-look if dragging and moved more than threshold I pulled
    // out of my butt aka 5 px
    if !drag_state.is_dragging || drag_state.drag_distance < 5.0 {
        return;
    }

    let Ok((entity, mut transform, mut free_look, orbit)) = camera_query.single_mut() else {
        return;
    };

    // Handle mouse motion first I guess, why not? Minot?
    if mouse.pressed(MouseButton::Left) {
        for event in mouse_motion.read() {
            if let Some(delta) = event.delta {
                // Block the rotation system from being a jerk whilst free looks on
                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;

                // Clamp pitch to prevent camera flips, its weird and I don't like it
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

    // Handle touch motion too cause ipads and whatnot I guess
    for event in touch_events.read() {
        if event.phase == bevy::input::touch::TouchPhase::Moved
            && Some(event.id) == drag_state.active_touch_id
        {
            if let Some(prev_pos) = drag_state.previous_pos {
                let delta = event.position - prev_pos;

                // blah blah no auto rotate here too
                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;

                // NEW COMMENTS FOR NO RAISIN
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

/// Nuke the FreeLookActive marker after 2 seconds and allow rotation again if
/// thats enabled cause y not
fn cleanup_free_look_after_inactivity(
    time: Res<Time>,
    query: Query<(Entity, &FreeLookActive)>,
    mut commands: Commands,
) {
    let current_time = time.elapsed_secs_f64();
    const INACTIVITY_THRESHOLD: f64 = 2.0;

    for (entity, free_look) in query.iter() {
        if current_time - free_look.0 > INACTIVITY_THRESHOLD {
            commands.entity(entity).remove::<FreeLookActive>();
        }
    }
}

/// Sync ColorState resource the background clear color
fn sync_color_state_to_clear_color(
    color_state: Res<ColorState>,
    mut clear_color: ResMut<ClearColor>,
) {
    if color_state.is_changed() {
        clear_color.0 = color_state.color.into();
    }
}
