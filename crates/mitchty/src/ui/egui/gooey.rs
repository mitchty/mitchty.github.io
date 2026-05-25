use bevy::prelude::*;
use bevy_egui::egui;

use super::{EguiMenuBarItem, ResetCamera};
use crate::CameraMode;
use crate::plugins::fps::{FpsDisplay, FpsTextRenderer};
use crate::plugins::hue::HueAnimation;
use crate::plugins::theme::theme_default_color;
use crate::ui::config::ThemeChoice;
use crate::ui::state::{MenuAnchor, UiPanel};

/// Plugin that registers the Gooey menu bar entry.
pub struct GooeyMenuPlugin;

impl Plugin for GooeyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "gooey",
                anchor: MenuAnchor::Left,
                order: 20,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Snapshot of Gooey-relevant ECS state, pre-extracted before the egui closure.
///
/// Mutable fields are written back to the owning resources by `settings_ui`
/// after `render_gooey_menu` returns the render function itself only reads
/// and writes this plain struct, keeping it free of Bevy resource borrows.
///
/// For now this is a glorified 2 phase commit across the ECS. Given the systems
/// run every tick we're at most 2 frames/ticks of the ECS away from a change
/// being visible on screen. Getting it to 1 is mostly unnecesary for this.
pub struct GooeyRenderData {
    /// If the fps thingy needs to display or not.
    pub fps_entity: Option<Entity>,
    /// Which renderer backs the FPS counter.
    pub fps_renderer: FpsTextRenderer,
    /// This is probably gonna get ripped out soon its not used.
    pub hue_entity: Option<Entity>,

    /// Toggle between Orthographic or normal projection modes.
    pub camera_mode: CameraMode,
    /// This is a hack, I'll do a future refactor for the projection to be a
    /// marker component.
    pub proj_toggle_requested: bool,

    /// If fullscreen shaders are enabled or not.
    pub effects_enabled: bool,
    /// Index into the available shaders list for active fullscreen shader to render with.
    pub active_shader_index: usize,
    /// Flat list of `(index, display_name)` for the shader pickers to abuse.
    pub shader_entries: Vec<(usize, String)>,

    /// `None` means "use theme default whatever the hell that is".
    pub color: Option<Srgba>,
    /// Here for when `color` is `None`.
    pub theme: ThemeChoice,
}

/// Render the Gooey drop-down menu entry into `ui`.
///
/// All mutations are recorded in `data` and written back by the caller after
/// the egui closure closes in a hacky af 2 phase commit approach so we don't
/// make the rust borrow gods angy. `reset_camera_events` is the one case where
/// we need to write an event immediately which is done via `MessageWriter`
/// since it is thread safe.
pub fn render_gooey_menu(
    ui: &mut egui::Ui,
    data: &mut GooeyRenderData,
    commands: &mut Commands,
    reset_camera_events: &mut bevy::prelude::MessageWriter<ResetCamera>,
) {
    ui.menu_button("Gooey", |ui| {
        ui.label(egui::RichText::new("Projection").strong());
        let proj_label = match data.camera_mode {
            CameraMode::Perspective => "Perspective [M]",
            CameraMode::Orthographic => "Orthographic [M]",
        };
        if ui.button(proj_label).clicked() {
            data.proj_toggle_requested = true;
        }
        if ui.button("Reset Camera").clicked() {
            reset_camera_events.write(ResetCamera);
            ui.close();
        }

        ui.separator();

        ui.label(egui::RichText::new("Effects").strong());
        if ui
            .checkbox(&mut data.effects_enabled, "Fullscreen Effect [E]")
            .changed()
        {
            // value already flipped in place and caller writes it back this is a nop in this context
        }
        ui.label("Shader:");
        for (idx, name) in &data.shader_entries {
            let selected = data.active_shader_index == *idx;
            if ui.selectable_label(selected, name).clicked() {
                data.active_shader_index = *idx;
            }
        }

        ui.separator();

        ui.label(egui::RichText::new("Toggles").strong());

        let mut fps_on = data.fps_entity.is_some();
        if ui.checkbox(&mut fps_on, "FPS Display [F]").changed() {
            if fps_on {
                commands.spawn(FpsDisplay);
            } else if let Some(e) = data.fps_entity {
                commands.entity(e).despawn();
            }
        }
        if fps_on {
            ui.horizontal(|ui| {
                ui.label("Renderer:");
                if ui
                    .selectable_label(data.fps_renderer == FpsTextRenderer::BevyText, "Bevy")
                    .clicked()
                {
                    data.fps_renderer = FpsTextRenderer::BevyText;
                }
                if ui
                    .selectable_label(data.fps_renderer == FpsTextRenderer::SlugText, "Slug")
                    .clicked()
                {
                    data.fps_renderer = FpsTextRenderer::SlugText;
                }
            });
        }

        let mut hue_on = data.hue_entity.is_some();
        if ui.checkbox(&mut hue_on, "Hue Animation [H]").changed() {
            if hue_on {
                commands.spawn(HueAnimation);
            } else if let Some(e) = data.hue_entity {
                commands.entity(e).despawn();
            }
        }

        ui.separator();

        ui.label(egui::RichText::new("Background color").strong());
        let display = data
            .color
            .unwrap_or_else(|| theme_default_color(data.theme));
        let mut color32 = egui::Color32::from_rgb(
            (display.red * 255.0) as u8,
            (display.green * 255.0) as u8,
            (display.blue * 255.0) as u8,
        );
        if egui::color_picker::color_picker_color32(
            ui,
            &mut color32,
            egui::color_picker::Alpha::Opaque,
        ) {
            let [r, g, b, _] = color32.to_normalized_gamma_f32();
            data.color = Some(bevy::color::Srgba::rgb(r, g, b));
        }
        if data.color.is_some() && ui.button("Reset to default").clicked() {
            data.color = None;
            ui.close();
        }
    });
}
