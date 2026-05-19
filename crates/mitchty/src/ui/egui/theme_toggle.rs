use bevy::prelude::*;
use bevy_egui::egui;

use super::EguiMenuBarItem;
use crate::plugins::theme::resolve_initial_theme;
use crate::ui::config::{ThemeChoice, UiConfig};
use crate::ui::state::{MenuAnchor, UiPanel};

/// Plugin that registers the theme-toggle button as a right-anchored menu bar entry.
pub struct ThemeToggleMenuPlugin;

impl Plugin for ThemeToggleMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "theme-toggle",
                anchor: MenuAnchor::Right,
                order: 0,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Render the 3-way theme toggle button 🌓  🌙  ☀ into `ui`.
///
/// Mutates `ui_config.theme` on click and immediately applies the new egui
/// visuals so there is no flash of the wrong theme between frames.
pub fn render_theme_toggle_menu(ui: &mut egui::Ui, ui_config: &mut UiConfig) {
    let (label, next) = match ui_config.theme {
        ThemeChoice::Auto => ("🌓", ThemeChoice::Dark),
        ThemeChoice::Dark => ("🌙", ThemeChoice::Light),
        ThemeChoice::Light => ("☀", ThemeChoice::Auto),
    };
    if ui.button(label).clicked() {
        ui_config.theme = next;
        let visuals = match next {
            ThemeChoice::Dark => egui::Visuals::dark(),
            ThemeChoice::Light => egui::Visuals::light(),
            ThemeChoice::Auto => resolve_initial_theme(ThemeChoice::Auto),
        };
        ui.ctx().set_visuals(visuals);
    }
}
