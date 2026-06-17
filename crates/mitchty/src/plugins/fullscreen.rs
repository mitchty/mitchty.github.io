//! Fullscreen post-process effect plugin, mostly enables fullscreen post
//! process fragment shaders to make things look spectacular. To me ok don't judge.
//!
//! Owns the camera orbit/config types, the keyboard toggles for cycling
//! through shader effects, and the per-frame time-uniform update.
//!
//! `CameraOrbit` and `CameraConfig` live here because this is the natural home
//! for everything that the old `fullscreen_effect` module contained. Ordering
//! in `main()` places `CameraPlugin` before `FullscreenEffectPlugin` so
//! `CameraConfig` is initialized in `build()` before any `Startup` system reads
//! it - no further sequencing is required.
// TODO: This layouts a bit "how you doing" but I'm not 100% sure if I want the
// camera plugin to own fullscreen effects or not. I'll sleep on it a bit this
// refactor is really stupid.

use bevy::prelude::*;

use crate::plugins::camera::{FreeLookCamera, MainCamera};
use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled, PostProcessSettings};

/// Marker component used by the feathers UI to track whether the fullscreen
/// post-process effect is enabled. Only meaningful under the `feathers`
/// feature; the egui path uses `EffectsEnabled` directly.
#[derive(Component, Default)]
pub struct FullscreenEffectEnabled;

/// Per-camera orbit data: the point the camera looks at and its orbital
/// radius.
#[derive(Component, Clone, Copy)]
pub struct CameraOrbit {
    pub center: Vec3,
    pub radius: f32,
}

/// Resource holding the full camera configuration used at spawn time.
#[derive(Resource, Clone, Copy)]
pub struct CameraConfig {
    pub transform: Transform,
    pub free_look: FreeLookCamera,
    pub orbit: CameraOrbit,
}

impl Default for CameraConfig {
    fn default() -> Self {
        // Position the camera so the default 0.3-scale GLB is nicely framed.
        // TODO: make this dynamic once scene AABB info is available at load time.
        let initial_pos = Vec3::new(3.0, 1.85, 3.0);
        let center = Vec3::new(0.0, 0.35, 0.0);
        let offset = initial_pos - center;
        let distance = offset.length();
        let yaw = offset.z.atan2(offset.x);
        let pitch = (offset.y / distance).asin();

        Self {
            transform: Transform::from_xyz(initial_pos.x, initial_pos.y, initial_pos.z)
                .looking_at(center, Vec3::Y),
            free_look: FreeLookCamera {
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

/// Toggle fullscreen effects on/off with the `E` key.
pub fn toggle_fullscreen_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    available_shaders: Res<AvailableShaders>,
    active_shader: Res<ActiveShader>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
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

/// Cycle forward through available shader effects with `.`.
pub fn next_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    effects_enabled: Res<EffectsEnabled>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::Period) {
        active_shader.next(&available_shaders);
        if effects_enabled.0 {
            debug!(
                "shader effect now {}",
                active_shader.display_name(&available_shaders)
            );
        }
    }
}

/// Cycle backward through available shader effects with `,`.
pub fn previous_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    effects_enabled: Res<EffectsEnabled>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::Comma) {
        active_shader.previous(&available_shaders);
        if effects_enabled.0 {
            debug!(
                "shader effect now {}",
                active_shader.display_name(&available_shaders)
            );
        }
    }
}

// TODO: I need to make gooey settings for each shader so I can dilly dally
// about with modifying values at runtime like intensity. This seems like a fun
// side quest in the future.
/// Sync the `PostProcessSettings.intensity` on the main camera to the current
/// `EffectsEnabled` / `ActiveShader` state.
pub fn manage_effect_settings(
    mut camera_query: Query<&mut PostProcessSettings, With<MainCamera>>,
    effects_enabled: Res<EffectsEnabled>,
    active_shader: Res<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
) {
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

pub struct FullscreenEffectPlugin;

impl Plugin for FullscreenEffectPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::plugins::camera::CameraPlugin>() {
            panic!("FullscreenEffectPlugin requires CameraPlugin to be added first!");
        }

        // CameraConfig is always needed as it owns the camera transform defaults.
        app.init_resource::<CameraConfig>();

        // The four systems below all take Res/ResMut of EffectsEnabled,
        // ActiveShader, and AvailableShaders are resources owned by
        // PostProcessPlugin. Skip them entirely when that plugin is disabled so
        // Bevy never tries to resolve the missing resources.
        if bavy::disabled::is_disabled(app.world(), "postprocess") {
            return;
        }

        app.world_mut().spawn(FullscreenEffectEnabled);
        app.add_systems(
            Update,
            (
                toggle_fullscreen_effect,
                next_effect,
                previous_effect,
                manage_effect_settings,
            ),
        );
    }
}
