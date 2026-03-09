use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled};
use crate::ui::scroll_view::{ActivePost, POSTS};
use crate::{ColorState, CubeRotation, DragState, FpsDisplay, HueAnimation};
use bevy::input::touch::TouchPhase;
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
            .add_systems(
                EguiPrimaryContextPass,
                (
                    configure_egui_style,
                    settings_ui,
                    update_egui_input_state,
                    toggle_egui,
                )
                    .chain(),
            );
    }
}

/// Bump up egui text by 2 points or so.
fn configure_egui_style(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    *done = true;

    contexts.ctx_mut()?.style_mut(|style| {
        for font_id in style.text_styles.values_mut() {
            font_id.size += 2.0;
        }
    });

    Ok(())
}

/// Spawn marker entities for egui state
fn setup_egui(mut commands: Commands, mut effects_enabled: ResMut<EffectsEnabled>) {
    // Start with effects enabled by default
    effects_enabled.0 = true;

    // Spawn other markers
    commands.spawn(CubeRotation);
    commands.spawn(HueAnimation);
    commands.spawn(FpsDisplay);

    // Show the menu bar by default... should I make this wasm only?
    commands.spawn(ShowEgui);
}

/// System to control the egui menu bar visibility.
/// g, mouse click, or touch toggles
fn toggle_egui(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut touch_events: MessageReader<TouchInput>,
    egui_entity: Query<Entity, With<ShowEgui>>,
    egui_wants_input: Res<EguiWantsInput>,
    drag_state: Res<DragState>,
    mut commands: Commands,
) {
    let keyboard_toggle = keyboard.just_pressed(KeyCode::KeyG);

    // A click or tap only counts if egui isn't consuming the pointer and the
    // pointer didn't travel far enough to be considered a drag/pan.
    let not_interacting = !egui_wants_input.wants_pointer;
    let not_dragging = drag_state.drag_distance < 5.0;

    let mouse_toggle = mouse.just_released(MouseButton::Left) && not_interacting && not_dragging;

    let touch_toggle = touch_events.read().any(|e| e.phase == TouchPhase::Ended)
        && not_interacting
        && not_dragging;

    if keyboard_toggle || mouse_toggle || touch_toggle {
        debug!("toggling egui menu bar");
        if let Ok(entity) = egui_entity.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(ShowEgui);
        }
    }
}

/// Display the settings UI using egui as a top menu bar
#[allow(clippy::too_many_arguments)]
fn settings_ui(
    mut contexts: EguiContexts,
    mut color_state: ResMut<ColorState>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    cube_rotation_query: Query<Entity, With<CubeRotation>>,
    hue_animation_query: Query<Entity, With<HueAnimation>>,
    show_egui_query: Query<(), With<ShowEgui>>,
    mut active_post: ResMut<ActivePost>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    mut commands: Commands,
) -> Result {
    if show_egui_query.is_empty() {
        return Ok(());
    }

    trace!("settings_ui running - ShowEgui exists");

    egui::TopBottomPanel::top("menu_bar").show(contexts.ctx_mut()?, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            // File menu, for now its just for a quit menu item on non wasm targets
            #[cfg(not(target_arch = "wasm32"))]
            ui.menu_button("File", |ui| {
                if ui.button("Quit").clicked() {
                    std::process::exit(0);
                }
            });

            // Clear color swatch pickerupper basically
            ui.menu_button("Background", |ui| {
                let mut color32 = egui::Color32::from_rgb(
                    (color_state.color.red * 255.0) as u8,
                    (color_state.color.green * 255.0) as u8,
                    (color_state.color.blue * 255.0) as u8,
                );
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut color32,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    let [r, g, b, _] = color32.to_normalized_gamma_f32();
                    color_state.color = bevy::color::Srgba::rgb(r, g, b);
                }
                if ui.button("Reset to Grey").clicked() {
                    color_state.color = bevy::color::Srgba::gray(0.5);
                    ui.close();
                }
            });

            // What dam shader to use or not, now just a menu item
            ui.menu_button("Effects", |ui| {
                let mut fullscreen_enabled = effects_enabled.0;
                if ui
                    .checkbox(&mut fullscreen_enabled, "Fullscreen Effect [E]")
                    .changed()
                {
                    effects_enabled.0 = fullscreen_enabled;
                }

                ui.separator();
                ui.label("Shader:");

                for (idx, shader_info) in available_shaders.shaders.iter().enumerate() {
                    let is_selected = active_shader.index == idx;
                    if ui
                        .selectable_label(is_selected, &shader_info.display_name)
                        .clicked()
                    {
                        active_shader.index = idx;
                        trace!(
                            "shader effect changed to {}",
                            active_shader.display_name(&available_shaders)
                        );
                    }
                }
            });

            // Toggleable toggles
            ui.menu_button("Toggles", |ui| {
                let mut fps_enabled = fps_query.single().is_ok();
                if ui.checkbox(&mut fps_enabled, "FPS Display [F]").changed() {
                    if fps_enabled {
                        commands.spawn(FpsDisplay);
                    } else if let Ok(entity) = fps_query.single() {
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

            ui.menu_button("Posts", |ui| {
                for (idx, post) in POSTS.iter().enumerate() {
                    let is_active = active_post.0 == Some(idx);
                    if ui.selectable_label(is_active, post.name).clicked() {
                        active_post.0 = if is_active { None } else { Some(idx) };
                        ui.close();
                    }
                }
            });
        });
    });
    Ok(())
}

/// System to update the EguiWantsInput resource based on egui's input state,
/// mostly here just to make sure egui input doesn't pass down to bevy.
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
