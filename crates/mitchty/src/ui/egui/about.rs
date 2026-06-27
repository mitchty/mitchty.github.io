use bevy::prelude::*;
use bevy_egui::egui;

use super::EguiMenuBarItem;
use crate::ui::state::{MenuAnchor, UiPanel};

pub struct AboutMenuPlugin;

impl Plugin for AboutMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "about",
                anchor: MenuAnchor::Right,
                order: 10,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Render the About drop-down menu entry into `ui`.
pub fn render_about_menu(ui: &mut egui::Ui, backend: Option<&str>) {
    ui.menu_button("About", |ui| {
        ui.hyperlink_to("GitHub Repo", lib::build_info::GIT_REPO);
        ui.separator();
        ui.label(format!("Version:  {}", env!("CARGO_PKG_VERSION")));
        if lib::build_info::GIT_DIRTY {
            ui.label(format!(
                "Commit:   {} modified",
                lib::build_info::GIT_COMMIT
            ));
        } else {
            ui.hyperlink_to(
                format!("Commit:   {}", lib::build_info::GIT_COMMIT),
                format!(
                    "{}/commit/{}",
                    lib::build_info::GIT_REPO,
                    lib::build_info::GIT_COMMIT
                ),
            );
        }
        ui.label(format!("Profile:  {}", lib::build_info::BUILD_PROFILE));
        ui.label(format!("Rustc:    {}", lib::build_info::RUSTC_VERSION));
        ui.label(format!("Backend:  {}", backend.unwrap_or("unknown")));
        ui.separator();
        ui.label("Third Party Acknowlegements");
        ui.separator();
        ui.label("Kanjivg");
        ui.hyperlink("https://kanjivg.tagaini.net");
    });
}
