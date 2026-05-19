use bevy::prelude::*;
use bevy_egui::egui;

use super::{EguiMenuBarItem, ShowSceneConfig};
use crate::ui::state::{MenuAnchor, UiPanel};

/// Plugin that registers the Scene menu bar entry.
///
/// The Scene label in the menu bar is a toggle for the `ShowSceneConfig` side
/// panel. The sidebar itself (`scene_config_window`) remains in `mod.rs` for
/// now cause I mostly just wanted a quick first pass to clean up the chungus
/// egui file.
pub struct SceneMenuPlugin;

impl Plugin for SceneMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "scene",
                anchor: MenuAnchor::Left,
                order: 10,
            },
            EguiMenuBarItem,
        ));
    }
}

pub struct SceneRenderData {
    /// `Some(entity)` when the Scene Config side panel should be open.
    pub scene_cfg_entity: Option<Entity>,
}

/// Render the Scene menu bar toggle into `ui`.
///
/// Clicking the label spawns or despawns `ShowSceneConfig`, which the
/// `scene_config_window` uses to render the side bar egui stuff.
pub fn render_scene_menu(ui: &mut egui::Ui, data: SceneRenderData, commands: &mut Commands) {
    let scene_cfg_open = data.scene_cfg_entity.is_some();
    if ui.selectable_label(scene_cfg_open, "Scene").clicked() {
        if let Some(entity) = data.scene_cfg_entity {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(ShowSceneConfig);
        }
    }
}
