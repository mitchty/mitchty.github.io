use bevy::prelude::*;
use bevy_egui::egui;
use mitchty::ActiveApp;

use super::EguiMenuBarItem;
use crate::ui::state::{MenuAnchor, UiPanel};
use crate::ui::world_clock::ShowWorldClock;

pub struct AppsMenuPlugin;

impl Plugin for AppsMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "apps",
                anchor: MenuAnchor::Left,
                order: 40,
            },
            EguiMenuBarItem,
        ));
    }
}

pub struct AppsRenderData {
    /// `Some(entity)` when the World Clock window is currently open.
    pub clock: Option<Entity>,
    /// Current active app variant for labels.
    pub active_app: ActiveApp,
}

/// Render the Apps drop-down menu entry into `ui`.
pub fn render_apps_menu(
    ui: &mut egui::Ui,
    data: AppsRenderData,
    commands: &mut Commands,
    active_app: &mut ActiveApp,
) {
    ui.menu_button("Apps", |ui| {
        ui.label(egui::RichText::new("Tooling").small().strong());
        ui.separator();

        let clock_open = data.clock.is_some();
        if ui.selectable_label(clock_open, "World Clock").clicked() {
            if let Some(entity) = data.clock {
                commands.entity(entity).despawn();
            } else {
                commands.spawn(ShowWorldClock);
            }
            ui.close();
        }

        ui.label(egui::RichText::new("Apps").small().strong());
        ui.separator();

        for variant in [ActiveApp::Default, ActiveApp::Jenga, ActiveApp::Pachinko] {
            let is_active = data.active_app == variant;
            if ui.selectable_label(is_active, variant.label()).clicked() && !is_active {
                *active_app = variant;
                ui.close();
            }
        }
    });
}
