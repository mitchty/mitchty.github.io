//! Hue animation plugin, largely a nop right now with scene gltf loading
// TODO: This too like the help text is probably on the chopping block.

use crate::plugins::text3d::Text3d;
use bevy::prelude::*;

/// Marker component that drives the hue animation on entities that have it.
#[derive(Component)]
pub struct HueAnimationEnabled;

/// Singleton marker controlling whether hue animation is globally active.
///
/// Spawn to enable, despawn to disable.
#[derive(Component, Default)]
pub struct HueAnimation;

/// Rotate the hue of every `StandardMaterial` on entities carrying
/// `HueAnimationEnabled`.
pub fn animate_materials(
    material_handles: Query<&MeshMaterial3d<StandardMaterial>, With<HueAnimationEnabled>>,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for material_handle in material_handles.iter() {
        if let Some(mut material) = materials.get_mut(material_handle)
            && let Color::Hsla(ref mut hsla) = material.base_color
        {
            *hsla = hsla.rotate_hue(time.delta_secs() * 100.0);
        }
    }
}

/// Toggle the `HueAnimation` global marker with the `H` key.
pub fn toggle_hue_animation(
    keyboard: Res<ButtonInput<KeyCode>>,
    hue_query: Query<Entity, With<HueAnimation>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyH) {
        if let Ok(entity) = hue_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(HueAnimation);
        }
    }
}

/// Propagate the `HueAnimation` global marker onto `Text3d` entities by
/// adding or removing `HueAnimationEnabled`.
pub fn apply_hue_animation(
    hue_marker: Query<(), With<HueAnimation>>,
    text3d_query: Query<(Entity, Has<HueAnimationEnabled>), With<Text3d>>,
    mut commands: Commands,
) {
    let should_animate = !hue_marker.is_empty();

    for (entity, has_animation) in text3d_query.iter() {
        if should_animate && !has_animation {
            commands.entity(entity).insert(HueAnimationEnabled);
        } else if !should_animate && has_animation {
            commands.entity(entity).remove::<HueAnimationEnabled>();
        }
    }
}

pub struct HuePlugin;

impl Plugin for HuePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            animate_materials.run_if(any_with_component::<HueAnimationEnabled>),
        )
        .add_systems(Update, (toggle_hue_animation, apply_hue_animation));
    }
}
