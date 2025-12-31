mod assets;
mod ui;

use assets::{AssetConfigPlugin, asset_path};
use bevy::prelude::*;
use rand::Rng;

use bevy_egui::EguiPlugin;
use bevy_old_tv_shader::prelude::*;
use ui::{SettingsUiPlugin, TvEffectEnabled};

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
#[derive(Component)]
pub struct CubeRotation;

/// Marker component to indicate hue animation is enabled (on cube entities)
#[derive(Component)]
pub struct HueAnimationEnabled;

/// Marker component separate from cube entities to control hue animation
#[derive(Component)]
pub struct HueAnimation;

/// Marker component for the FPS text entity
#[derive(Component)]
struct FpsText;

/// Marker component to indicate FPS should be displayed and updated
#[derive(Component)]
pub struct FpsDisplay;

/// Marker component to indicate camera should be rotating
#[derive(Component)]
pub struct CameraRotationEnabled;

/// Marker component for the main camera to enable TV effect toggling
#[derive(Component)]
pub struct MainCamera;

/// Resource for the tv shader settings, for future maybe to make it so I can
/// change stuff in it dynamically.
#[derive(Resource)]
pub struct TvSettingsResource {
    pub settings: OldTvSettings,
}

/// Marker component for camera rotation
#[derive(Component)]
struct RotatingCamera {
    /// Rotation speed in radians per second (positive = clockwise when viewed from above)
    speed: f32,
    /// Radius of rotation around the center
    radius: f32,
    /// Center to rotate around
    center: Vec3,
    /// Current angle in radians
    angle: f32,
    /// Height of the camera
    height: f32,
}

fn main() {
    // Set up better panic messages for WASM for when this stuff seems to not
    // work or I manage to use a library that won't run on it without paying
    // attention... again.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(assets::create_default_plugins())
        .add_plugins(AssetConfigPlugin)
        .add_plugins(OldTvPlugin)
        .add_plugins(EguiPlugin::default())
        .add_plugins(SettingsUiPlugin)
        .insert_resource(ClearColor(Color::WHITE))
        .add_systems(Startup, (setup, setup_fps_ui))
        .add_systems(
            Update,
            (
                animate_materials.run_if(any_with_component::<HueAnimationEnabled>),
                rotate_entities.run_if(any_with_component::<CubeRotationEnabled>),
                rotate_camera.run_if(any_with_component::<CameraRotationEnabled>),
                toggle_fps_display,
                toggle_camera_rotation,
                toggle_cube_rotation,
                toggle_hue_animation,
                toggle_tv_effect,
                apply_tv_effect,
                apply_camera_rotation,
                apply_cube_rotation,
                apply_hue_animation,
                update_fps_display.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs_f32(0.5),
                )),
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let diffuse_path = asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    let specular_path = asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");

    #[allow(clippy::field_reassign_with_default)]
    let tv_settings = {
        let mut tv_settings = OldTvSettings::default();
        tv_settings.screen_shape_factor = 0.2;
        tv_settings.rows = 112.0;
        tv_settings.brightness = 3.0;
        tv_settings.edges_transition_size = 0.025;
        tv_settings.channels_mask_min = 0.1;
        tv_settings
    };

    let initial_pos = Vec3::new(3.0, 1.0, 3.0);
    let center = Vec3::new(0.0, -0.5, 0.0);

    commands.insert_resource(TvSettingsResource {
        settings: tv_settings,
    });

    commands.spawn((
        Camera3d::default(),
        tv_settings,
        Transform::from_xyz(initial_pos.x, initial_pos.y, initial_pos.z)
            .looking_at(center, Vec3::Y),
        EnvironmentMapLight {
            diffuse_map: asset_server.load(diffuse_path),
            specular_map: asset_server.load(specular_path),
            intensity: 2_000.0,
            ..default()
        },
        RotatingCamera {
            speed: 0.3,
            radius: (initial_pos.x.powi(2) + initial_pos.z.powi(2)).sqrt(),
            center,
            angle: initial_pos.z.atan2(initial_pos.x),
            height: initial_pos.y,
        },
        CameraRotationEnabled,
        MainCamera,
    ));

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

/// Camera rotation state
#[derive(Component)]
pub struct CameraRotation;

/// Rotation of the camera around the origin/center point
fn rotate_camera(
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut RotatingCamera), With<CameraRotationEnabled>>,
) {
    for (mut transform, mut camera) in query.iter_mut() {
        camera.angle += camera.speed * time.delta_secs();

        let x = camera.center.x + camera.radius * camera.angle.cos();
        let z = camera.center.z + camera.radius * camera.angle.sin();

        transform.translation = Vec3::new(x, camera.height, z);

        *transform = transform.looking_at(camera.center, Vec3::Y);
    }
}

/// Toggle camera rotation
/// r toggles on/off
fn toggle_camera_rotation(
    keyboard: Res<ButtonInput<KeyCode>>,
    rotation_query: Query<Entity, With<CameraRotation>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyR) {
        if let Ok(entity) = rotation_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(CameraRotation);
        }
    }
}

/// System to apply camera rotation.
// TODO: rename toggle_*?
fn apply_camera_rotation(
    rotation_marker: Query<(), With<CameraRotation>>,
    camera_query: Query<(Entity, Has<CameraRotationEnabled>), With<RotatingCamera>>,
    mut commands: Commands,
) {
    let should_rotate = !rotation_marker.is_empty();

    for (entity, has_rotation) in camera_query.iter() {
        if should_rotate && !has_rotation {
            commands.entity(entity).insert(CameraRotationEnabled);
        } else if !should_rotate && has_rotation {
            commands.entity(entity).remove::<CameraRotationEnabled>();
        }
    }
}

/// System to spawn the fps text entity
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
    // Only update if FpsDisplay marker exists
    if fps_display_query.is_empty() {
        // Clear the text when FPS display is off
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

/// Toggle TV effect on the main camera
/// t toggles on/off
fn toggle_tv_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    tv_effect_query: Query<Entity, With<TvEffectEnabled>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyT) {
        if let Ok(entity) = tv_effect_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(TvEffectEnabled);
        }
    }
}

/// Apply or remove TV effect toggle
fn apply_tv_effect(
    tv_effect_query: Query<(), With<TvEffectEnabled>>,
    camera_query: Query<(Entity, Has<OldTvSettings>), With<MainCamera>>,
    tv_settings: Res<TvSettingsResource>,
    mut commands: Commands,
) {
    let tv_should_be_enabled = !tv_effect_query.is_empty();

    for (entity, has_tv_settings) in camera_query.iter() {
        if tv_should_be_enabled && !has_tv_settings {
            commands.entity(entity).insert(tv_settings.settings);
        } else if !tv_should_be_enabled && has_tv_settings {
            commands.entity(entity).remove::<OldTvSettings>();
        }
    }
}

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

/// Apply or remove CubeRotationEnabled component toggle
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
