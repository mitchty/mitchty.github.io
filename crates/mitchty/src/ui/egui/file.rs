use bevy::prelude::*;

use super::EguiMenuBarItem;
use crate::ui::state::{MenuAnchor, UiPanel};

/// Plugin that registers the File menu bar entry.
///
/// This is native only.
pub struct FileMenuPlugin;

impl Plugin for FileMenuPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(not(target_arch = "wasm32"))]
        app.world_mut().spawn((
            UiPanel {
                id: "file",
                anchor: MenuAnchor::Left,
                order: 0,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Render the File drop-down menu entry into `ui`. Native only.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_file_menu(ui: &mut bevy_egui::egui::Ui) {
    ui.menu_button("File", |ui| {
        if ui.button("Quit").clicked() {
            std::process::exit(0);
        }
    });
}
