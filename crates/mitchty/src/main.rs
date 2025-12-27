mod assets;

use assets::{AssetConfigPlugin, asset_path};
use bevy::prelude::*;

/// Marker component for entities that should rotate
#[derive(Component)]
struct Rotator {
    /// Rotation speed in radians per second
    speed: f32,
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

    const GOLDEN_ANGLE: f32 = 137.507_77;

    let mut hsla = Hsla::hsl(0.0, 1.0, 0.5);
    for x in -1..2 {
        for z in -1..2 {
            commands.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(materials.add(Color::from(hsla))),
                Transform::from_translation(Vec3::new(x as f32, 0.0, z as f32)),
                Rotator { speed: 1.0 }, // Rotate at 1 radian per second
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

fn rotate_entities(mut query: Query<(&mut Transform, &Rotator)>, time: Res<Time>) {
    for (mut transform, rotator) in &mut query {
        transform.rotate_y(rotator.speed * time.delta_secs());
    }
}
