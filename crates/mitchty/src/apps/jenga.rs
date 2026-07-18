//! Jenga physics app.
use avian3d::prelude::*;
use bevy::prelude::*;
use mitchty::ActiveApp;

use crate::plugins::camera::MainCamera;

// TODO: As much as I love const, and I love it a lot, I should make this stuff
// a Resource so I can modify it at runtime in the ecs same as pachinko now too.
// Eh later.
const BALL_RADIUS: f32 = 0.08;
/// How fast the dodgeball is yeeted. This is probably too much but its fun to
/// watch it ricochet so it stays.
const BALL_SPEED: f32 = 40.0;

/// Spawn offset in front of the camera so the ball doesn't start literally at
/// the camera origin which looks weird af ngl.
const BALL_SPAWN_OFFSET: f32 = 0.5;

/// Half extents of a Jenga block in xyz, "real" jenga is apparently 75mm × 25mm
/// × 15mm and since we size the world in meters its fine, just slightly scaled
/// up in the world view
const BLOCK_HALF: Vec3 = Vec3::new(0.225, 0.075, 0.075);

/// Gap between blocks in the same row so they don't interact needlessly
const BLOCK_GAP: f32 = 0.005;

/// Number of block layer pairs each pair = 2 cross cutting rows.
const N_LAYERS: u32 = 10;

const FLOOR_Y: f32 = 0.0;

#[derive(Component)]
pub struct JengaWorld;

#[derive(Component)]
pub struct JengaBlock;

#[derive(Component)]
pub struct JengaFloor;

// If you can dodge a wrench, you can dodge a ball, unless you're a static jenga
// tower in which case you're kinda boned and at the mercy of the user.
#[derive(Component)]
pub struct JengaDodgeBall;

/// Spawn the floor and jenga tower.
pub fn spawn_jenga_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor_half_size = Vec3::new(4.0, 0.05, 4.0);
    commands.spawn((
        JengaWorld,
        JengaFloor,
        RigidBody::Static,
        Collider::cuboid(
            floor_half_size.x * 2.0,
            floor_half_size.y * 2.0,
            floor_half_size.z * 2.0,
        ),
        Mesh3d(meshes.add(Cuboid::new(
            floor_half_size.x * 2.0,
            floor_half_size.y * 2.0,
            floor_half_size.z * 2.0,
        ))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.3, 0.3),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_xyz(0.0, FLOOR_Y - floor_half_size.y, 0.0),
    ));

    // Put a point light above the tower so it doesn't look like shadow ass
    commands.spawn((
        JengaWorld,
        PointLight {
            intensity: 100_000.0,
            range: 20.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 6.0, 0.0),
    ));

    // This is just two slightly different brown tones so the blocks don't look
    // the same edge on.
    let mat_a = materials.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.67, 0.44),
        perceptual_roughness: 0.7,
        ..default()
    });
    let mat_b = materials.add(StandardMaterial {
        base_color: Color::srgb(0.72, 0.56, 0.34),
        perceptual_roughness: 0.75,
        ..default()
    });

    let block_mesh = meshes.add(Cuboid::new(
        BLOCK_HALF.x * 2.0,
        BLOCK_HALF.y * 2.0,
        BLOCK_HALF.z * 2.0,
    ));

    // Collider extents for avian.
    let block_collider =
        Collider::cuboid(BLOCK_HALF.x * 2.0, BLOCK_HALF.y * 2.0, BLOCK_HALF.z * 2.0);

    let row_height = BLOCK_HALF.y * 2.0;

    // Note blocks are spaced side by side on their shorter axis which is z in
    // bevy land.
    let block_pitch = BLOCK_HALF.z * 2.0 + BLOCK_GAP;

    let offsets = [-block_pitch, 0.0, block_pitch];

    for layer in 0..N_LAYERS {
        // Base Y of the current layer blocks, sitting on top of the floor to start
        let y = FLOOR_Y + row_height * (layer as f32 + 0.5);
        let mat = if layer % 2 == 0 {
            mat_a.clone()
        } else {
            mat_b.clone()
        };

        // Even layers long axis is X, odd is Y
        let (rotation, x_off, z_off): (Quat, f32, f32) = if layer % 2 == 0 {
            (Quat::IDENTITY, 0.0, 1.0)
        } else {
            (Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), 1.0, 0.0)
        };

        for (i, &offset) in offsets.iter().enumerate() {
            let pos = Vec3::new(offset * x_off, y, offset * z_off);

            commands.spawn((
                JengaWorld,
                JengaBlock,
                RigidBody::Dynamic,
                block_collider.clone(),
                Friction::new(0.8),
                Restitution::new(0.1),
                Mesh3d(block_mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform {
                    translation: pos,
                    rotation,
                    ..default()
                },
                // Give alternate blocks a different material color
                Name::new(format!("JengaBlock L{layer}B{i}")),
            ));
        }
    }
}

/// Fire a ball from the camera toward where it's looking.
pub fn fire_ball(
    keyboard: Res<ButtonInput<KeyCode>>,
    camera_query: Query<&Transform, With<MainCamera>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    // WHEN WILL MY MISERY AROUND EGUI END.
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }

    if !keyboard.just_pressed(KeyCode::Space) {
        return;
    }

    let Ok(cam_transform) = camera_query.single() else {
        return;
    };

    let forward = cam_transform.forward();
    let spawn_pos = cam_transform.translation + forward * BALL_SPAWN_OFFSET;
    let velocity = forward * BALL_SPEED;

    commands.spawn((
        JengaWorld,
        JengaDodgeBall,
        RigidBody::Dynamic,
        Collider::sphere(BALL_RADIUS),
        Restitution::new(0.4),
        Friction::new(0.5),
        LinearVelocity(velocity),
        Mesh3d(meshes.add(Sphere::new(BALL_RADIUS))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.2, 0.2),
            perceptual_roughness: 0.3,
            metallic: 0.1,
            ..default()
        })),
        Transform::from_translation(spawn_pos),
        Name::new("JengaDodgeBall"),
    ));
}

pub fn despawn_jenga_world(query: Query<Entity, With<JengaWorld>>, mut commands: Commands) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

/// Plugin that owns the Jenga experience.
///
/// Registers `PhysicsPlugins` (Avian) unconditionally — Avian is cheap when
/// there are no rigid bodies, so keeping it always-on is fine.
///
/// The `spawn` / `despawn` systems are gated on `ActiveApp` changes so they
/// only fire when entering / leaving the Jenga app.
pub struct JengaPlugin;

impl Plugin for JengaPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PhysicsSchedulePlugin>() {
            app.add_plugins(PhysicsPlugins::default());
        }

        app.init_resource::<mitchty::ActiveApp>();

        // Fire a dodgeball towards the camera's aim point.
        app.add_systems(Update, fire_ball.run_if(resource_equals(ActiveApp::Jenga)));

        app.add_systems(
            PostUpdate,
            spawn_jenga_world
                .run_if(resource_changed::<ActiveApp>.and_then(resource_equals(ActiveApp::Jenga))),
        );

        app.add_systems(
            PostUpdate,
            despawn_jenga_world.run_if(not(resource_equals(ActiveApp::Jenga))),
        );
    }
}
