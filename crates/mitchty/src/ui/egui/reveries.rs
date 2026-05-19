use bevy::prelude::*;
use bevy_egui::egui;

use super::EguiMenuBarItem;
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName, ReverieKey, reveries_egui_menu};
use crate::ui::state::{MenuAnchor, UiPanel};

/// Plugin that registers the Reveries menu bar entry.
///
/// This for now mostly just the egui menu bar item and the display bits are all
/// bevy ui so nothing here for that. Thats the eventual goal once feathers/bsn is ready.
pub struct ReveriesMenuPlugin;

impl Plugin for ReveriesMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "reveries",
                anchor: MenuAnchor::Left,
                order: 30,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Render the Reveries drop-down menu entry into `ui`.
pub fn render_reveries_menu(
    ui: &mut egui::Ui,
    entries: &[(Entity, &ReverieKey, &ReverieDisplayName)],
    active_reverie: &mut ActiveReverie,
) {
    reveries_egui_menu(ui, entries, active_reverie);
}
