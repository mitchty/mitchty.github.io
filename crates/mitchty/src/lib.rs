// TODO: DRY MORE SLACKER
use bevy::prelude::*;

/// Camera projection mode aka perspective/orthographic
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CameraMode {
    #[default]
    Perspective,
    Orthographic,
}

/// Which "app" experience is currently active.
///
/// `Default`  = The full mitchty experience of.. not a lot suckers.
/// `Jenga`    = A dum Jenga tower you can knock over with a crappy dodgeball.
/// `Pachinko` = A dum Pachinko attempt for no reason.
///
/// Changing this resource is the top-level routing signal — each app plugin
/// watches it and spawns/despawns its own world accordingly.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ActiveApp {
    #[default]
    Default,
    Jenga,
    Pachinko,
}

impl ActiveApp {
    pub fn label(self) -> &'static str {
        match self {
            ActiveApp::Default => "mitchty",
            ActiveApp::Jenga => "Jenga",
            ActiveApp::Pachinko => "Pachinko",
        }
    }
}

// Re-export cause lazy
pub use bevy::camera::visibility::RenderLayers;
pub use bevy::prelude::Camera3d;
pub use bevy::prelude::Projection;
