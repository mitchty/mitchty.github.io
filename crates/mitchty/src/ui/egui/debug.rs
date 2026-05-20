use bevy::prelude::*;

use super::EguiMenuBarItem;
use crate::ui::state::{MenuAnchor, UiPanel};

pub struct DebugMenuPlugin;

impl Plugin for DebugMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "debug",
                anchor: MenuAnchor::Right,
                order: 20,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Render the Debug drop-down menu into `ui`.
///
/// Lists every entry in `PluginRegistry` as a live checkbox so plugins can be
/// toggled at runtime without recompiling. `sync_registry_to_plugins` in
/// `PreUpdate` propagates the flag changes to the actual `PluginEnabled<T>`
/// resources on the next tick.
pub fn render_debug_menu(
    ui: &mut bevy_egui::egui::Ui,
    registry: &mut crate::plugins::PluginRegistry,
) {
    ui.menu_button("Debug", |ui| {
        ui.label(bevy_egui::egui::RichText::new("Plugin Toggles").strong());
        ui.separator();
        if registry.entries.is_empty() {
            ui.label(
                bevy_egui::egui::RichText::new("No plugins registered.")
                    .italics()
                    .weak(),
            );
        } else {
            for entry in registry.entries.iter_mut() {
                ui.checkbox(&mut entry.enabled, entry.name);
            }
        }
    });
}
