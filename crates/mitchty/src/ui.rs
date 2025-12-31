use crate::{CameraRotation, CubeRotation, FpsDisplay, HueAnimation};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

/// Marker component to indicate egui UI should be displayed
#[derive(Component)]
pub struct ShowEgui;

/// Marker component to indicate TV effect is enabled
#[derive(Component)]
pub struct TvEffectEnabled;

/// Plugin for egui UI
pub struct SettingsUiPlugin;

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_egui)
            .add_systems(Update, toggle_egui)
            .add_systems(
                EguiPrimaryContextPass,
                settings_ui.run_if(any_with_component::<ShowEgui>),
            );
    }
}

/// Spawn marker entities for egui state
fn setup_egui(mut commands: Commands) {
    // Don't spawn ShowEgui by default - egui starts hidden
    commands.spawn(TvEffectEnabled);
    commands.spawn(CameraRotation);
    commands.spawn(CubeRotation);
    commands.spawn(HueAnimation);
}

/// System to control the egui settings/debug panel visibility
/// d or touch (for things like ipad/wasm builds) toggles
fn toggle_egui(
    keyboard: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    egui_entity: Query<Entity, With<ShowEgui>>,
    mut commands: Commands,
) {
    let should_toggle = keyboard.just_pressed(KeyCode::KeyD) || touches.any_just_pressed();

    if should_toggle {
        if let Ok(entity) = egui_entity.single() {
            commands.entity(entity).remove::<ShowEgui>();
        } else {
            // If no entity has ShowEgui, spawn one
            commands.spawn(ShowEgui);
        }
    }
}

/// Display the settings UI using egui
#[allow(clippy::too_many_arguments)]
fn settings_ui(
    mut contexts: EguiContexts,
    mut clear_color: ResMut<ClearColor>,
    tv_effect_query: Query<Entity, With<TvEffectEnabled>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    camera_rotation_query: Query<Entity, With<CameraRotation>>,
    cube_rotation_query: Query<Entity, With<CubeRotation>>,
    hue_animation_query: Query<Entity, With<HueAnimation>>,
    mut commands: Commands,
) -> Result {
    egui::Window::new("Debug").show(contexts.ctx_mut()?, |ui| {
        ui.heading("Clear Color");

        // Convert Bevy Color to egui color array
        let bevy_color = clear_color.0.to_srgba();
        let mut color = [bevy_color.red, bevy_color.green, bevy_color.blue];

        // Color picker
        if ui.color_edit_button_rgb(&mut color).changed() {
            clear_color.0 = Color::srgb(color[0], color[1], color[2]);
        }

        // Reset button
        if ui.button("Reset to White").clicked() {
            clear_color.0 = Color::WHITE;
        }

        ui.separator();
        ui.heading("Effects");

        // TV Effect checkbox
        let mut tv_enabled = tv_effect_query.single().is_ok();
        if ui.checkbox(&mut tv_enabled, "TV Effect").changed() {
            if tv_enabled {
                commands.spawn(TvEffectEnabled);
            } else if let Ok(entity) = tv_effect_query.single() {
                commands.entity(entity).despawn();
            }
        }

        // FPS Display checkbox
        let mut fps_enabled = fps_query.single().is_ok();
        if ui.checkbox(&mut fps_enabled, "FPS Display").changed() {
            if fps_enabled {
                commands.spawn(FpsDisplay);
            } else if let Ok(entity) = fps_query.single() {
                commands.entity(entity).despawn();
            }
        }

        // Camera Rotation checkbox
        let mut camera_rotation_enabled = camera_rotation_query.single().is_ok();
        if ui
            .checkbox(&mut camera_rotation_enabled, "Camera Rotation")
            .changed()
        {
            if camera_rotation_enabled {
                commands.spawn(CameraRotation);
            } else if let Ok(entity) = camera_rotation_query.single() {
                commands.entity(entity).despawn();
            }
        }

        // Cube Rotation checkbox
        let mut cube_rotation_enabled = cube_rotation_query.single().is_ok();
        if ui
            .checkbox(&mut cube_rotation_enabled, "Cube Rotation")
            .changed()
        {
            if cube_rotation_enabled {
                commands.spawn(CubeRotation);
            } else if let Ok(entity) = cube_rotation_query.single() {
                commands.entity(entity).despawn();
            }
        }

        // Hue Animation checkbox
        let mut hue_animation_enabled = hue_animation_query.single().is_ok();
        if ui
            .checkbox(&mut hue_animation_enabled, "Hue Animation")
            .changed()
        {
            if hue_animation_enabled {
                commands.spawn(HueAnimation);
            } else if let Ok(entity) = hue_animation_query.single() {
                commands.entity(entity).despawn();
            }
        }
    });
    Ok(())
}
