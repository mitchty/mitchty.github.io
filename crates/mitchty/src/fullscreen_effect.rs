// TODO: better name for this in future fullscreenshader maybe
use bevy::{
    core_pipeline::{core_3d::graph::Node3d, fullscreen_material::FullscreenMaterial},
    prelude::*,
    render::{
        extract_component::ExtractComponent,
        render_graph::{InternedRenderLabel, RenderLabel},
        render_resource::ShaderType,
    },
    shader::ShaderRef,
};

use crate::asset_path_raw;

/// Fullscreen effect shader struct. Future me can figure out how to maybe
/// generate multiple? Eh whatever.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
pub struct FullscreenEffect {
    /// Intensity of the effect, basically treat it like an alpha channel 0. =
    /// transparent 1. = opaque
    pub intensity: f32,
    pub time: f32,
    #[cfg(target_arch = "wasm32")]
    _webgl2_padding: Vec2,
}

impl FullscreenMaterial for FullscreenEffect {
    // TODO: brain up a way to wire the asset server shaders into a dropdown
    // with bevy ui if thats possible? This is a future mitch task that past
    // mitch has zero regrets pawning off to future mitch.
    fn fragment_shader() -> ShaderRef {
        asset_path_raw!("shaders/em-interference.wgsl").into()
    }

    fn node_edges() -> Vec<InternedRenderLabel> {
        vec![
            Node3d::Tonemapping.intern(),
            Self::node_label().intern(),
            Node3d::EndMainPassPostProcessing.intern(),
        ]
    }
}

#[derive(Component, Default)]
pub struct FullscreenEffectEnabled;

/// Toggle fullscreen effect
/// e toggles on/off
pub fn toggle_fullscreen_effect(
    keyboard: Res<ButtonInput<KeyCode>>,
    effect_query: Query<Entity, With<FullscreenEffectEnabled>>,
    mut commands: Commands,
) {
    if keyboard.just_pressed(KeyCode::KeyE) {
        if let Ok(entity) = effect_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(FullscreenEffectEnabled);
        }
    }
}

/// Apply or disable fullscreen effect on the main camera
/// Note: We don't remove the component to avoid render graph state issues, that
/// led to... unique results lets just say. I need to figure out how the full
/// screen shader works internally cause adding it and removing it was a bad
/// idea. Did produce some interesting output at least. If ugly.
///
/// So abuse the 0.0/1.0 nonsense to cover if something should be active or not.
/// Future task is making it truly like alpha channels.
pub fn apply_fullscreen_effect(
    effect_marker: Query<(), With<FullscreenEffectEnabled>>,
    mut camera_query: Query<(Entity, Option<&mut FullscreenEffect>), With<crate::MainCamera>>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let should_enable = !effect_marker.is_empty();
    let current_time = time.elapsed_secs();

    for (entity, effect) in camera_query.iter_mut() {
        match effect {
            Some(mut effect) => {
                effect.intensity = if should_enable { 1.0 } else { 0.0 };
                effect.time = current_time;
            }
            None => {
                if should_enable {
                    commands.entity(entity).insert(FullscreenEffect {
                        intensity: 1.0,
                        time: current_time,
                        #[cfg(target_arch = "wasm32")]
                        _webgl2_padding: Default::default(),
                    });
                }
            }
        }
    }
}

/// Is this needed? I thought I can get time in shaders but I barely know what
/// I'm doing with graphics at the best of times. Whatever it worked passing it
/// into the shader so ship it.
pub fn update_fullscreen_effect_time(
    time: Res<Time>,
    mut effect_query: Query<&mut FullscreenEffect>,
) {
    let current_time = time.elapsed_secs();
    for mut effect in effect_query.iter_mut() {
        effect.time = current_time;
    }
}
