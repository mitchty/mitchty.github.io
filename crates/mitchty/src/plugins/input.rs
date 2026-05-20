//! Top-level input plugin.
//!
//! Handles keyboard shortcuts that don't belong to a more specific plugin:
//! currently just `M` to toggle camera projection.
//!
//! Mouse/touch drag input lives in `camera`, FPS toggle in `fps`, hue toggle in
//! `hue`. For now.... this was a very quick hack refactor without much thought.
// TODO: Brain up a better overall approach to input and stop being lazy. Maybe
// this just goes into the camera... The more I tried to make this fetch happen
// it didn't work well. Likely means a rethink.
use crate::ui::ToggleCameraProjection;
use bevy::prelude::*;

/// Send a `ToggleCameraProjection` message on `M` keypress.
pub fn toggle_camera_projection(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut events: MessageWriter<ToggleCameraProjection>,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyM) {
        events.write(ToggleCameraProjection);
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_camera_projection);
    }
}
