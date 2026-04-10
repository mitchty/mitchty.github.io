// TODO: DRY MORE SLACKER
use bevy::prelude::*;

/// Camera projection mode aka perspective/orthographic
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CameraMode {
    #[default]
    Perspective,
    Orthographic,
}

// Re-export cause lazy
pub use bevy::camera::visibility::RenderLayers;
pub use bevy::prelude::Camera3d;
pub use bevy::prelude::Projection;
