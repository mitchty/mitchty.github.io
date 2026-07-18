//! Pachinko machine app.
//!
use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::prelude::*;
use mitchty::ActiveApp;

use crate::plugins::camera::{FreeLookCamera, MainCamera};
use crate::plugins::fullscreen::CameraOrbit;

const BOARD_WIDTH: f32 = 3.0;
const BOARD_HEIGHT: f32 = 5.5;
const BOARD_DEPTH: f32 = 0.24;
/// Thickness of every static wall.
const WALL_T: f32 = 0.1;
/// Peg radius.
const PEG_R: f32 = 0.055;
/// Ball radius should be smaller than ^^^.
const BALL_R: f32 = 0.048;
/// Seconds between the next ball drop.
const SPAWN_SECS: f32 = 0.5;
/// How many buckets for the bottom of the board.
const BUCKET_COUNT: usize = 5;
/// How far in front of the board the camera sits by default.
const CAM_DIST: f32 = 8.0;
/// Bounciness shared by pegs and wall surfaces.
const BOUNCE: f32 = 0.55;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallColor {
    Red = 0,
    Green = 1,
    Blue = 2,
}

impl BallColor {
    fn random() -> Self {
        match rand::random_range(0u32..3) {
            0 => Self::Red,
            1 => Self::Green,
            _ => Self::Blue,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Red => Color::srgb(0.95, 0.18, 0.18),
            Self::Green => Color::srgb(0.18, 0.88, 0.25),
            Self::Blue => Color::srgb(0.18, 0.35, 0.98),
        }
    }
}

/// Per bucket and colour hit counts.
#[derive(Resource, Default)]
pub struct BucketScores {
    pub counts: [[u32; 3]; BUCKET_COUNT],
}

impl BucketScores {
    fn total(&self, b: usize) -> u32 {
        self.counts[b].iter().sum()
    }

    fn pct(&self, b: usize, c: usize) -> f32 {
        let t = self.total(b);
        if t == 0 {
            0.0
        } else {
            self.counts[b][c] as f32 / t as f32 * 100.0
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Resource)]
pub struct BallSpawnTimer(pub Timer);

/// Whether the ball spawner is currently running. Toggled by spacebar while in
/// Pachinko mode to pause balls.
#[derive(Resource, Default)]
pub struct PachinkoSpawning(pub bool);

/// Tags every entity owned by the pachinko world used for despawn. This
/// includes UI nodes as well so everyting gets despawned on app switch.
#[derive(Component)]
pub struct PachinkoWorld;

/// Which value a score cell displays.
#[derive(Component, Clone, Copy)]
pub enum ScoreCell {
    /// Total ball count for this bucket.
    Total { bucket: usize },
    /// Percentage of a given color (0=R, 1=G, 2=B) in this bucket.
    ColorPct { bucket: usize, color: usize },
}

#[derive(Component)]
pub struct PachinkoBucket {
    pub index: usize,
}

#[derive(Component)]
pub struct PachinkoBall {
    pub color: BallColor,
}

#[derive(Component)]
pub struct PachinkoControlsHint;

/// Static visible physics wall.
fn wall(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: Handle<StandardMaterial>,
    pos: Vec3,
    size: Vec3,
) {
    commands.spawn((
        PachinkoWorld,
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Restitution::new(BOUNCE),
        Friction::new(0.15),
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(mat),
        Transform::from_translation(pos),
    ));
}

// The front of the pachinko machine so we can see whats happening, this isn't
// Shroedingers box we'd like to see what is happening.
fn invisible_wall(commands: &mut Commands, pos: Vec3, size: Vec3) {
    commands.spawn((
        PachinkoWorld,
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Restitution::new(BOUNCE),
        Friction::new(0.15),
        Transform::from_translation(pos),
    ));
}

pub fn spawn_pachinko_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut scores: ResMut<BucketScores>,
    mut timer: ResMut<BallSpawnTimer>,
    mut spawning: ResMut<PachinkoSpawning>,
    mut cam_q: Query<(&mut Transform, &mut FreeLookCamera, &mut CameraOrbit), With<MainCamera>>,
) {
    scores.reset();
    timer.0.reset();
    spawning.0 = true;

    // Camera faces the board front on. TODO: Need to setup a reset camera later.
    if let Ok((mut tf, mut look, mut orbit)) = cam_q.single_mut() {
        orbit.center = Vec3::new(0.0, BOARD_HEIGHT * 0.5, 0.0);
        orbit.radius = CAM_DIST;
        look.yaw = FRAC_PI_2;
        look.pitch = 0.0;
        let c = orbit.center;
        let r = orbit.radius;
        tf.translation = Vec3::new(
            c.x + r * look.yaw.cos() * look.pitch.cos(),
            c.y + r * look.pitch.sin(),
            c.z + r * look.yaw.sin() * look.pitch.cos(),
        );
        *tf = tf.looking_at(orbit.center, Vec3::Y);
    }

    let wall_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.25, 0.30),
        perceptual_roughness: 0.9,
        ..default()
    });
    let peg_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.75, 0.75, 0.78),
        perceptual_roughness: 0.3,
        metallic: 0.7,
        ..default()
    });

    let hw = BOARD_WIDTH * 0.5;
    let hh = BOARD_HEIGHT * 0.5;
    let hd = BOARD_DEPTH * 0.5;

    // Left
    wall(
        &mut commands,
        &mut meshes,
        wall_mat.clone(),
        Vec3::new(-hw - WALL_T * 0.5, hh, 0.0),
        Vec3::new(WALL_T, BOARD_HEIGHT + WALL_T * 2.0, BOARD_DEPTH),
    );
    // Right
    wall(
        &mut commands,
        &mut meshes,
        wall_mat.clone(),
        Vec3::new(hw + WALL_T * 0.5, hh, 0.0),
        Vec3::new(WALL_T, BOARD_HEIGHT + WALL_T * 2.0, BOARD_DEPTH),
    );
    // Bottom
    wall(
        &mut commands,
        &mut meshes,
        wall_mat.clone(),
        Vec3::new(0.0, -WALL_T * 0.5, 0.0),
        Vec3::new(BOARD_WIDTH + WALL_T * 2.0, WALL_T, BOARD_DEPTH),
    );
    // Front has not mesh so we can see through it.
    invisible_wall(
        &mut commands,
        Vec3::new(0.0, hh, hd + WALL_T * 0.5),
        Vec3::new(
            BOARD_WIDTH + WALL_T * 2.0,
            BOARD_HEIGHT + WALL_T * 2.0,
            WALL_T,
        ),
    );
    // Back
    wall(
        &mut commands,
        &mut meshes,
        wall_mat.clone(),
        Vec3::new(0.0, hh, -hd - WALL_T * 0.5),
        Vec3::new(
            BOARD_WIDTH + WALL_T * 2.0,
            BOARD_HEIGHT + WALL_T * 2.0,
            WALL_T,
        ),
    );

    // Peg grid with staggered rows, I haven't looked if this is sane...ish.
    // Plus I want to induce randomness into this pig later anyway. Pegs are
    // cylinders for a cross-section in XY coordinate space for ball bounces.
    // Cylinders span the full board depth along the Z axis. Bevy/Avian
    // cylinders default to the Y-axis, so rotate them on X so the length is at
    // Z.
    let peg_rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let peg_mesh = meshes.add(Cylinder::new(PEG_R, BOARD_DEPTH));
    let step_x = 0.35_f32;
    let step_y = 0.38_f32;
    let y_start = 0.65_f32;
    let y_end = BOARD_HEIGHT - 0.85;
    let mut row = 0_u32;
    let mut py = y_start;
    while py <= y_end + 0.001 {
        // Even rows start on the left, odd rows are offset by half a step so it staggers.
        let x_off = if row.is_multiple_of(2) {
            0.0
        } else {
            step_x * 0.5
        };
        let x_start = -hw + step_x;
        let x_end = hw - step_x;
        let mut px = x_start + x_off;
        while px <= x_end + 0.001 {
            commands.spawn((
                PachinkoWorld,
                RigidBody::Static,
                Collider::cylinder(PEG_R, BOARD_DEPTH),
                Restitution::new(BOUNCE),
                Friction::new(0.05),
                Mesh3d(peg_mesh.clone()),
                MeshMaterial3d(peg_mat.clone()),
                Transform::from_xyz(px, py, 0.0).with_rotation(peg_rotation),
            ));
            px += step_x;
        }
        py += step_y;
        row += 1;
    }

    // Scoring buckets spawns, the sensors are at the bottom.
    let bucket_w = BOARD_WIDTH / BUCKET_COUNT as f32;
    let bucket_h = 0.4_f32;
    // Divider height slightly taller than bucket so balls don't skip over until
    // you fill em in which case yeah don't do that or hit r to reset.
    let divider_h = bucket_h + 0.12;

    // Tint each bucket individually, should make this dynamic at some point.
    // TODO: future mitch ^^^
    let bucket_tints = [
        Color::srgba(0.80, 0.20, 0.20, 0.35),
        Color::srgba(0.85, 0.55, 0.10, 0.35),
        Color::srgba(0.20, 0.75, 0.20, 0.35),
        Color::srgba(0.20, 0.40, 0.90, 0.35),
        Color::srgba(0.65, 0.15, 0.80, 0.35),
    ];

    let divider_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.40, 0.40, 0.45),
        perceptual_roughness: 0.8,
        ..default()
    });

    for (i, tint) in bucket_tints.iter().enumerate() {
        let bx = -hw + bucket_w * (i as f32 + 0.5);
        let by = bucket_h * 0.5;

        // Ball sensor balls pass through but generate collision events to track
        // stuff, not perfect but it'll do.
        commands.spawn((
            PachinkoWorld,
            PachinkoBucket { index: i },
            RigidBody::Static,
            Sensor,
            Collider::cuboid(bucket_w - 0.02, bucket_h, BOARD_DEPTH),
            CollisionEventsEnabled,
            Mesh3d(meshes.add(Cuboid::new(bucket_w - 0.02, bucket_h, BOARD_DEPTH))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: *tint,
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(bx, by, 0.0),
        ));

        // Divider wall between buckets but the last bucket.
        if i < BUCKET_COUNT - 1 {
            let dx = -hw + bucket_w * (i as f32 + 1.0);
            wall(
                &mut commands,
                &mut meshes,
                divider_mat.clone(),
                Vec3::new(dx, divider_h * 0.5, 0.0),
                Vec3::new(0.025, divider_h, BOARD_DEPTH),
            );
        }
    }

    commands.spawn((
        PachinkoWorld,
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(3.0, 8.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

pub fn spawn_pachinko_balls(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut timer: ResMut<BallSpawnTimer>,
    time: Res<Time>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let color = BallColor::random();

    // Random horizontal entry point minus the side walls which would be a silly spot to spawn.
    let margin = BALL_R * 3.0 + WALL_T;
    let x = rand::random_range((-BOARD_WIDTH * 0.5 + margin)..(BOARD_WIDTH * 0.5 - margin));
    let y = BOARD_HEIGHT - BALL_R - 0.05;

    // Tiny yeet the balls down a bit so they don't hover weirdly at spawn. We
    // want our balls to hang down after all.
    let vel = Vec3::new(rand::random_range(-0.3_f32..0.3), -0.4, 0.0);

    commands.spawn((
        PachinkoWorld,
        PachinkoBall { color },
        RigidBody::Dynamic,
        Collider::sphere(BALL_R),
        CollisionEventsEnabled,
        Restitution::new(0.50),
        Friction::new(0.10),
        LinearVelocity(vel),
        Mesh3d(meshes.add(Sphere::new(BALL_R))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color.color(),
            // Slight emissive glow so the balls are visible even if they're not directly getting a light source.
            emissive: (color.color().to_linear() * 0.4),
            perceptual_roughness: 0.25,
            metallic: 0.15,
            ..default()
        })),
        Transform::from_xyz(x, y, 0.0),
        Name::new(format!("PachinkoBall({color:?})")),
    ));
}

pub fn score_pachinko(
    mut events: MessageReader<CollisionStart>,
    bucket_q: Query<&PachinkoBucket>,
    ball_q: Query<&PachinkoBall>,
    mut scores: ResMut<BucketScores>,
) {
    for event in events.read() {
        let (e1, e2) = (event.collider1, event.collider2);
        // Figure out which entity is the ball or the bucket we don't know which
        // is which.
        let hit = bucket_q
            .get(e1)
            .ok()
            .zip(ball_q.get(e2).ok())
            .or_else(|| bucket_q.get(e2).ok().zip(ball_q.get(e1).ok()));

        if let Some((bucket, ball)) = hit {
            scores.counts[bucket.index][ball.color as usize] += 1;
        }
    }
}

pub fn despawn_pachinko_world(
    query: Query<Entity, With<PachinkoWorld>>,
    mut commands: Commands,
    mut scores: ResMut<BucketScores>,
    mut spawning: ResMut<PachinkoSpawning>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
    scores.reset();
    spawning.0 = false;
}

/// Toggle ball spawning with spacebar presses. I'll figure out what to do for
/// touch devices too hot/tired right now to care.
pub fn toggle_pachinko_spawning(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut spawning: ResMut<PachinkoSpawning>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::Space) {
        spawning.0 = !spawning.0;
    }
}

/// Despawn every live ball and reset all bucket scores when 'R' is pressed.
pub fn reset_pachinko_balls(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    ball_q: Query<Entity, With<PachinkoBall>>,
    mut scores: ResMut<BucketScores>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyR) {
        for entity in ball_q.iter() {
            commands.entity(entity).despawn();
        }
        scores.reset();
    }
}

const LABEL_W: f32 = 30.0;
const CELL_W: f32 = 40.0;
const SCORE_FONT: f32 = 13.0;

/// A single score-table row "type" to shut clippy up about complexity its not a
/// problem here this is for fun.
type ScoreRowSpec = (&'static str, Color, fn(usize) -> ScoreCell);

pub fn spawn_score_ui(mut commands: Commands) {
    let white = Color::WHITE;
    let grey = Color::srgb(0.6, 0.6, 0.6);
    let red_c = Color::srgb(0.95, 0.35, 0.35);
    let green_c = Color::srgb(0.30, 0.90, 0.35);
    let blue_c = Color::srgb(0.35, 0.55, 0.98);

    // Outer panel is absolute, top-left, dark semi-transparent node.
    commands
        .spawn((
            PachinkoWorld,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(40.0), // leave room for the menu bar need to use bevy immediate at some point
                left: Val::Px(8.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.82)),
        ))
        .with_children(|panel| {
            panel.spawn((
                PachinkoWorld,
                Text::new("Pachinko Scores"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(SCORE_FONT + 1.0),
                    ..default()
                },
                TextColor(white),
            ));

            // Header row: blank label | B1 B2 B3 B4 B5
            panel
                .spawn((
                    PachinkoWorld,
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(2.0),
                        ..default()
                    },
                ))
                .with_children(|row| {
                    row.spawn((
                        PachinkoWorld,
                        Text::new(""),
                        TextFont {
                            font_size: bevy::text::FontSize::Px(SCORE_FONT),
                            ..default()
                        },
                        Node {
                            width: Val::Px(LABEL_W),
                            ..default()
                        },
                    ));
                    for i in 0..BUCKET_COUNT {
                        row.spawn((
                            PachinkoWorld,
                            Text::new(format!("B{}", i + 1)),
                            TextFont {
                                font_size: bevy::text::FontSize::Px(SCORE_FONT),
                                ..default()
                            },
                            TextColor(grey),
                            Node {
                                width: Val::Px(CELL_W),
                                ..default()
                            },
                        ));
                    }
                });

            // Data rows: (label, color, per bucket score)
            let rows: &[ScoreRowSpec] = &[
                ("n", white, |b| ScoreCell::Total { bucket: b }),
                ("R%", red_c, |b| ScoreCell::ColorPct {
                    bucket: b,
                    color: 0,
                }),
                ("G%", green_c, |b| ScoreCell::ColorPct {
                    bucket: b,
                    color: 1,
                }),
                ("B%", blue_c, |b| ScoreCell::ColorPct {
                    bucket: b,
                    color: 2,
                }),
            ];

            for &(label, label_color, cell_fn) in rows {
                panel
                    .spawn((
                        PachinkoWorld,
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(2.0),
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        row.spawn((
                            PachinkoWorld,
                            Text::new(label),
                            TextFont {
                                font_size: bevy::text::FontSize::Px(SCORE_FONT),
                                ..default()
                            },
                            TextColor(label_color),
                            Node {
                                width: Val::Px(LABEL_W),
                                ..default()
                            },
                        ));
                        // One data cell per bucket starts with "  " until a ball scores.
                        for b in 0..BUCKET_COUNT {
                            row.spawn((
                                PachinkoWorld,
                                cell_fn(b),
                                Text::new("--"),
                                TextFont {
                                    font_size: bevy::text::FontSize::Px(SCORE_FONT),
                                    ..default()
                                },
                                TextColor(label_color),
                                Node {
                                    width: Val::Px(CELL_W),
                                    ..default()
                                },
                            ));
                        }
                    });
            }
        });
}

/// Refresh every `ScoreCell` text node whenever `BucketScores` changes.
pub fn update_score_ui(scores: Res<BucketScores>, mut cells: Query<(&ScoreCell, &mut Text)>) {
    if !scores.is_changed() {
        return;
    }
    for (cell, mut text) in cells.iter_mut() {
        *text = Text::new(match *cell {
            ScoreCell::Total { bucket } => format!("{}", scores.total(bucket)),
            ScoreCell::ColorPct { bucket, color } => {
                let pct = scores.pct(bucket, color);
                if scores.total(bucket) == 0 {
                    "--".to_string()
                } else {
                    format!("{pct:.0}%")
                }
            }
        });
    }
}

/// Spawn a brief controls-hint overlay when entering Pachinko mode so peeps
/// know what they can do.
pub fn spawn_controls_hint(mut commands: Commands) {
    commands.spawn((
        PachinkoWorld,
        PachinkoControlsHint,
        Text::new("Space to pause or resume balls R/r to reset"),
        TextFont {
            font_size: bevy::text::FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(80.0),
            left: Val::Percent(10.0),
            top: Val::Percent(50.0),
            ..default()
        },
    ));
}

/// Despawn the controls hint on the first touch, mouse-click, or key-press so
/// it doesn't get in the way.
pub fn dismiss_controls_hint_on_input(
    mut touch_events: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    hint_query: Query<Entity, With<PachinkoControlsHint>>,
    mut commands: Commands,
) {
    let touched = touch_events.read().next().is_some();
    let clicked = mouse.get_just_pressed().next().is_some();
    let key_pressed = keyboard.get_just_pressed().next().is_some();

    if touched || clicked || key_pressed {
        for entity in hint_query.iter() {
            commands.entity(entity).despawn();
        }
    }
}

pub struct PachinkoPlugin;

impl Plugin for PachinkoPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PhysicsSchedulePlugin>() {
            app.add_plugins(PhysicsPlugins::default());
        }

        app.init_resource::<BucketScores>()
            .init_resource::<PachinkoSpawning>()
            .insert_resource(BallSpawnTimer(Timer::from_seconds(
                SPAWN_SECS,
                TimerMode::Repeating,
            )));

        app.add_systems(
            PostUpdate,
            spawn_pachinko_world.run_if(
                resource_changed::<ActiveApp>.and_then(resource_equals(ActiveApp::Pachinko)),
            ),
        );

        app.add_systems(
            Update,
            toggle_pachinko_spawning.run_if(resource_equals(ActiveApp::Pachinko)),
        );

        app.add_systems(
            Update,
            reset_pachinko_balls.run_if(resource_equals(ActiveApp::Pachinko)),
        );

        app.add_systems(
            Update,
            spawn_pachinko_balls.run_if(
                resource_equals(ActiveApp::Pachinko).and_then(|s: Res<PachinkoSpawning>| s.0),
            ),
        );

        // Scoring runs always so we can detect before/after pauses.
        app.add_systems(
            Update,
            score_pachinko.run_if(resource_equals(ActiveApp::Pachinko)),
        );

        // Score UI panel is last after anything updates cause that makes the most sense.
        app.add_systems(
            PostUpdate,
            spawn_score_ui.run_if(
                resource_changed::<ActiveApp>.and_then(resource_equals(ActiveApp::Pachinko)),
            ),
        );

        // Refresh score text cells whenever BucketScores changes, might be
        // worth making an observer or message?
        app.add_systems(
            Update,
            update_score_ui.run_if(resource_equals(ActiveApp::Pachinko)),
        );

        app.add_systems(
            PostUpdate,
            spawn_controls_hint.run_if(
                resource_changed::<ActiveApp>.and_then(resource_equals(ActiveApp::Pachinko)),
            ),
        );
        app.add_systems(
            Update,
            dismiss_controls_hint_on_input.run_if(resource_equals(ActiveApp::Pachinko)),
        );

        app.add_systems(
            PostUpdate,
            despawn_pachinko_world.run_if(not(resource_equals(ActiveApp::Pachinko))),
        );
    }
}
