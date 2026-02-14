use crate::fullscreen_effect::FullscreenEffectEnabled;
use crate::{CameraRotation, ColorState, CubeRotation, FpsDisplay, HueAnimation};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

/// Resource to track if egui is currently using input, helps with accidental
/// clicks not bleeding downwards to bevy.
#[derive(Resource, Default)]
pub struct EguiWantsInput {
    pub wants_pointer: bool,
    pub wants_keyboard: bool,
}

/// Marker component for the egui ui display
#[derive(Component)]
pub struct ShowEgui;

/// Plugin for egui UI
pub struct SettingsUiPlugin;

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EguiWantsInput>()
            .add_systems(Startup, setup_egui)
            .add_systems(Update, toggle_egui)
            .add_systems(
                EguiPrimaryContextPass,
                (settings_ui, update_egui_input_state).chain(),
            );
    }
}

/// Spawn marker entities for egui state
fn setup_egui(mut commands: Commands) {
    // Start with these enabled
    commands.spawn(ShowEgui);
    commands.spawn(FullscreenEffectEnabled);
    commands.spawn(CameraRotation);
    commands.spawn(CubeRotation);
    commands.spawn(HueAnimation);
    commands.spawn(FpsDisplay);
}

/// System to control the egui settings/debug panel visibility
/// g or touch (for things like ipad/wasm builds) toggles
fn toggle_egui(
    keyboard: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    egui_entity: Query<Entity, With<ShowEgui>>,
    mut commands: Commands,
) {
    let should_toggle = keyboard.just_pressed(KeyCode::KeyG) || touches.any_just_pressed();

    if should_toggle {
        if let Ok(entity) = egui_entity.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(ShowEgui);
        }
    }
}

/// Display the settings UI using egui
#[allow(clippy::too_many_arguments)]
fn settings_ui(
    mut contexts: EguiContexts,
    mut color_state: ResMut<ColorState>,
    fullscreen_query: Query<Entity, With<FullscreenEffectEnabled>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    camera_rotation_query: Query<Entity, With<CameraRotation>>,
    cube_rotation_query: Query<Entity, With<CubeRotation>>,
    hue_animation_query: Query<Entity, With<HueAnimation>>,
    show_egui_query: Query<(), With<ShowEgui>>,
    mut commands: Commands,
) -> Result {
    if show_egui_query.is_empty() {
        return Ok(());
    }

    egui::SidePanel::left("settings_panel")
        .default_width(250.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.heading("Background Color");

            let mut color = [
                color_state.color.red,
                color_state.color.green,
                color_state.color.blue,
            ];

            if ui.color_edit_button_rgb(&mut color).changed() {
                color_state.color = bevy::color::Srgba::rgb(color[0], color[1], color[2]);
            }

            if ui.button("Reset to Grey").clicked() {
                color_state.color = bevy::color::Srgba::gray(0.5);
            }

            ui.separator();
            ui.heading("Effects");

            let mut fullscreen_enabled = fullscreen_query.single().is_ok();
            if ui
                .checkbox(&mut fullscreen_enabled, "Fullscreen Effect [E]")
                .changed()
            {
                if fullscreen_enabled {
                    commands.spawn(FullscreenEffectEnabled);
                } else if let Ok(entity) = fullscreen_query.single() {
                    commands.entity(entity).despawn();
                }
            }

            let mut fps_enabled = fps_query.single().is_ok();
            if ui.checkbox(&mut fps_enabled, "FPS Display [F]").changed() {
                if fps_enabled {
                    commands.spawn(FpsDisplay);
                } else if let Ok(entity) = fps_query.single() {
                    commands.entity(entity).despawn();
                }
            }

            let mut camera_rotation_enabled = camera_rotation_query.single().is_ok();
            if ui
                .checkbox(&mut camera_rotation_enabled, "Camera Rotation [R]")
                .changed()
            {
                if camera_rotation_enabled {
                    commands.spawn(CameraRotation);
                } else if let Ok(entity) = camera_rotation_query.single() {
                    commands.entity(entity).despawn();
                }
            }

            let mut cube_rotation_enabled = cube_rotation_query.single().is_ok();
            if ui
                .checkbox(&mut cube_rotation_enabled, "Cube Rotation [C]")
                .changed()
            {
                if cube_rotation_enabled {
                    commands.spawn(CubeRotation);
                } else if let Ok(entity) = cube_rotation_query.single() {
                    commands.entity(entity).despawn();
                }
            }

            let mut hue_animation_enabled = hue_animation_query.single().is_ok();
            if ui
                .checkbox(&mut hue_animation_enabled, "Hue Animation [H]")
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

/// System to update the EguiWantsInput resource based on egui's input state
/// This runs after the UI is drawn and helps other systems know if egui is using input
fn update_egui_input_state(
    mut contexts: EguiContexts,
    mut egui_wants_input: ResMut<EguiWantsInput>,
    show_egui_query: Query<(), With<ShowEgui>>,
) -> Result {
    if show_egui_query.is_empty() {
        egui_wants_input.wants_pointer = false;
        egui_wants_input.wants_keyboard = false;
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    // Update the resource with current egui input state
    // This includes clicks, drags, and mouse wheel when over the egui panel...
    // I think, might be missing crap here. I don't know how to program gooeys.
    egui_wants_input.wants_pointer = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
    egui_wants_input.wants_keyboard = ctx.wants_keyboard_input();

    Ok(())
}
