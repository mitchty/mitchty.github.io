use bevy::prelude::*;
use bevy_egui::egui;

use super::{EguiMenuBarItem, ShowLosant, ShowRecognizer};
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::data_viewer::ShowDataViewer;
use crate::ui::state::{MenuAnchor, UiPanel};

pub struct ExperimentsMenuPlugin;

impl Plugin for ExperimentsMenuPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().spawn((
            UiPanel {
                id: "experiments",
                anchor: MenuAnchor::Right,
                order: 30,
            },
            EguiMenuBarItem,
        ));
    }
}

/// Pre-extracted state for the Experiments render function.
///
/// Built by `settings_ui` from its system params before the egui closure opens,
/// then passed through. This sidesteps the borrow-checker issue of accessing
/// `marker_queries` inside a closure that also captures `commands`.
pub struct ExperimentsRenderData {
    /// `Some(entity)` when the `ShowRecognizer` window marker exists.
    pub recognizer_entity: Option<Entity>,
    /// `Some((entity, is_visible))` for the flan `PlotUiNode`, or `None` if
    /// the sparkline node doesn't exist yet.
    pub line_graph: Option<(Entity, bool)>,
    /// `Some(entity)` when `ShowDataViewer` is present. Native only cause I'll
    /// be damned if I figure out how to get browser wasm to be able to open
    /// files. Plus these things are gigs of size.
    #[cfg(not(target_arch = "wasm32"))]
    pub data_viewer_entity: Option<Entity>,
    /// `Some(entity)` when the Losant window marker exists.
    pub losant_entity: Option<Entity>,
}

/// Render the Experiments drop-down menu entry into `ui`.
///
/// Uses pre-extracted entity data so it has no direct query access. Visibility
/// changes are applied via ECS `Commands` rather than mutating the `Visibility`
/// component in place so that the ECS logic is separate from any gooey code.
///
/// This will be useful later, I hope.
pub fn render_experiments_menu(
    ui: &mut egui::Ui,
    data: ExperimentsRenderData,
    commands: &mut Commands,
) {
    ui.menu_button("Experiments", |ui| {
        ui.label(egui::RichText::new("Abominable Intelligence").strong());
        ui.separator();

        if ui.button("Recognizer").clicked() {
            if let Some(entity) = data.recognizer_entity {
                commands.entity(entity).despawn();
            } else {
                commands.spawn(ShowRecognizer);
            }
            ui.close();
        }

        #[cfg(not(target_arch = "wasm32"))]
        if ui.button("Data Viewer").clicked() {
            if let Some(entity) = data.data_viewer_entity {
                commands.entity(entity).despawn();
            } else {
                commands.spawn(ShowDataViewer);
            }
            ui.close();
        }

        ui.separator();

        ui.label(egui::RichText::new("Flan Shaders").strong());

        let line_graph_visible = data.line_graph.map(|(_, v)| v).unwrap_or(false);
        if ui
            .selectable_label(line_graph_visible, "Line Graph")
            .clicked()
        {
            if let Some((entity, visible)) = data.line_graph {
                commands.entity(entity).insert(if visible {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                });
            }
            ui.close();
        }

        ui.separator();

        ui.label(egui::RichText::new("Embedded").strong());

        let losant_open = data.losant_entity.is_some();
        if ui.selectable_label(losant_open, "Losant").clicked() {
            if let Some(entity) = data.losant_entity {
                commands.entity(entity).despawn();
            } else {
                commands.spawn(ShowLosant);
            }
            ui.close();
        }
    });
}
