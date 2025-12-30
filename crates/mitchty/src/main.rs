mod assets;

use assets::{AssetConfigPlugin, asset_path};
use bevy::prelude::*;
use rand::Rng;

use bevy_old_tv_shader::prelude::*;

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

/// Marker component for the FPS text entity
#[derive(Component)]
struct FpsText;

/// Marker component to indicate FPS should be displayed and updated
#[derive(Component)]
struct ShowFps;

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
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, (setup, setup_fps_ui))
        .add_systems(
            Update,
            (
                animate_materials,
                rotate_entities,
                toggle_fps_display,
                update_fps_display
                    .run_if(any_with_component::<ShowFps>)
                    .run_if(bevy::time::common_conditions::on_timer(
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
        tv_settings.screen_shape_factor = 0.3;
        tv_settings.rows = 192.0;
        tv_settings.brightness = 3.0;
        tv_settings.edges_transition_size = 0.025;
        tv_settings.channels_mask_min = 0.1;
        tv_settings
    };

    commands.spawn((
        Camera3d::default(),
        tv_settings,
        Transform::from_xyz(3.0, 1.0, 3.0).looking_at(Vec3::new(0.0, -0.5, 0.0), Vec3::Y),
        EnvironmentMapLight {
            diffuse_map: asset_server.load(diffuse_path),
            specular_map: asset_server.load(specular_path),
            intensity: 2_000.0,
            ..default()
        },
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
            ));
            hsla = hsla.rotate_hue(GOLDEN_ANGLE);
        }
    }
}

/// System to enable the cube materials to animate over time
fn animate_materials(
    material_handles: Query<&MeshMaterial3d<StandardMaterial>>,
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

/// System to rotate all the cubes over every axis independently and randomly
fn rotate_entities(mut query: Query<(&mut Transform, &mut Rotator)>, time: Res<Time>) {
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

/// System to start an text overlay for fps
fn setup_fps_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("0.0 fps"),
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
        Visibility::Hidden,
        FpsText,
    ));
}

/// Toggle ShowFps marker component to control systems that display the fps text
/// Keyboard input of f toggles this on/off. The marker entity is not created by default.
fn toggle_fps_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    fps_text_query: Query<(Entity, Has<ShowFps>), With<FpsText>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyF) {
        for (entity, has_show_fps) in fps_text_query.iter() {
            if has_show_fps {
                commands.entity(entity).remove::<ShowFps>();
                commands.entity(entity).insert(Visibility::Hidden);
            } else {
                commands.entity(entity).insert(ShowFps);
                commands.entity(entity).insert(Visibility::Visible);
            }
        }
    }
}

// Iff we have the ShowFps marker component hanging around, update the fps text.
fn update_fps_display(
    time: Res<Time>,
    mut fps_text_query: Query<&mut Text, (With<FpsText>, With<ShowFps>)>,
) {
    let fps = 1.0 / time.delta_secs();

    for mut text in fps_text_query.iter_mut() {
        text.0 = format!("{:.1} fps", fps);
    }
}
