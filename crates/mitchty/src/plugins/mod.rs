pub mod camera;
pub mod cli;
pub mod disabled;
pub mod fonts;
pub mod fps;
pub mod fullscreen;
pub mod help;
pub mod hue;
pub mod input;
pub mod reveries;
pub mod scene;
pub mod terminal;
pub mod text3d;
pub mod theme;
pub mod toggle;

// TODO: Get these working again.
#[allow(unused_imports)]
pub use toggle::run_if_disabled;
pub use toggle::{PluginEnabled, PluginRegistry, run_if_enabled, sync_registry_to_plugins};
