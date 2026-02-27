// Fullscreen post-processing effect management
use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled, PostProcessSettings};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

/// Camera orbit data for where the camera is pointing.
#[derive(Component, Clone, Copy)]
pub struct CameraOrbit {
    pub center: Vec3,
    pub radius: f32,
}

/// Camera configuration data that needs to be preserved across camera swaps
#[derive(Resource, Clone, Copy)]
pub struct CameraConfig {
    pub transform: Transform,
    pub free_look: crate::FreeLookCamera,
    pub orbit: CameraOrbit,
}

impl Default for CameraConfig {
    fn default() -> Self {
        // TODO: Should make this calculation to center the 3d stuff to be
        // dynamic, future me problem.
        let initial_pos = Vec3::new(3.0, 1.85, 3.0);
        let center = Vec3::new(0.0, 0.35, 0.0);
        let offset = initial_pos - center;
        let distance = offset.length();
        let yaw = offset.z.atan2(offset.x);
        let pitch = (offset.y / distance).asin();

        Self {
            transform: Transform::from_xyz(initial_pos.x, initial_pos.y, initial_pos.z)
                .looking_at(center, Vec3::Y),
            free_look: crate::FreeLookCamera {
                yaw,
                pitch,
                sensitivity: 0.003,
            },
            orbit: CameraOrbit {
                center,
                radius: distance,
            },
        }
    }
}

/// Toggle fullscreen effects on/off
/// e toggles on/off
pub fn toggle_fullscreen_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    available_shaders: Res<AvailableShaders>,
    active_shader: Res<ActiveShader>,
) {
    if keyboard.just_pressed(KeyCode::KeyE) {
        effects_enabled.0 = !effects_enabled.0;
        let status = if effects_enabled.0 {
            "enabled"
        } else {
            "disabled"
        };
        debug!(
            "effects {}: {}",
            status,
            active_shader.display_name(&available_shaders)
        );
    }
}

/// Cycle to next effect
/// . cycles forward for now
pub fn next_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    effects_enabled: Res<EffectsEnabled>,
) {
    if keyboard.just_pressed(KeyCode::Period) {
        active_shader.next(&available_shaders);
        if effects_enabled.0 {
            debug!(
                "effect == {}",
                active_shader.display_name(&available_shaders)
            );
        }
    }
}

/// Cycle to back to a prior effect
/// , cycles backward
pub fn previous_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    effects_enabled: Res<EffectsEnabled>,
) {
    if keyboard.just_pressed(KeyCode::Comma) {
        active_shader.previous(&available_shaders);
        if effects_enabled.0 {
            debug!(
                "effect ==  {}",
                active_shader.display_name(&available_shaders)
            );
        }
    }
}

/// Spawn camera with post-processing settings
pub fn spawn_camera(
    mut commands: Commands,
    config: Res<CameraConfig>,
    asset_server: Res<AssetServer>,
    effects_enabled: Res<EffectsEnabled>,
) {
    let diffuse_path = crate::asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    let specular_path = crate::asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");

    let intensity = if effects_enabled.0 { 1.0 } else { 0.0 };

    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1, // Render before default (0) - 3D scene renders first
            ..default()
        },
        config.transform,
        EnvironmentMapLight {
            diffuse_map: asset_server.load(diffuse_path),
            specular_map: asset_server.load(specular_path),
            intensity: 2_000.0,
            ..default()
        },
        config.free_look,
        config.orbit,
        crate::MainCamera,
        PostProcessSettings {
            intensity,
            time: 0.0,
            #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
            _webgl2_padding: Vec2::ZERO,
        },
        // Only render layer 0 (main 3D scene)
        // Layer 1 is for overlays without post-processing
        RenderLayers::layer(0),
    ));

    debug!("camera post-processing {}", effects_enabled.0);
}

/// Update post-processing settings based on enabled state
pub fn manage_effect_settings(
    mut camera_query: Query<&mut PostProcessSettings, With<crate::MainCamera>>,
    effects_enabled: Res<EffectsEnabled>,
    active_shader: Res<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
) {
    // Only run when something changes
    if !effects_enabled.is_changed() && !active_shader.is_changed() {
        return;
    }

    let Ok(mut settings) = camera_query.single_mut() else {
        return;
    };

    let new_intensity = if effects_enabled.0 { 1.0 } else { 0.0 };

    if settings.intensity != new_intensity {
        settings.intensity = new_intensity;

        if effects_enabled.0 {
            debug!(
                "enabled effect: {}",
                active_shader.display_name(&available_shaders)
            );
        } else {
            debug!("all post processing effects disabled");
        }
    }
}

/// Update time uniform for animated effects
pub fn update_effect_time(time: Res<Time>, mut settings_query: Query<&mut PostProcessSettings>) {
    let current_time = time.elapsed_secs();

    for mut settings in settings_query.iter_mut() {
        settings.time = current_time;
    }
}
