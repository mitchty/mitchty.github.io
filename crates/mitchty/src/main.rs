mod assets;

use assets::{AssetConfigPlugin, asset_path};
use bevy::prelude::*;
use rand::Rng;

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

fn main() {
    // Set up better panic messages for WASM for when this stuff seems to not
    // work or I manage to use a library that won't run on it without paying
    // attention... again.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(assets::create_default_plugins())
        .add_plugins(AssetConfigPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_systems(Update, (animate_materials, rotate_entities))
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

    commands.spawn((
        Camera3d::default(),
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
