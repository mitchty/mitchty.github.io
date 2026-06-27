pub mod canary;
pub mod fullscreen;
pub mod plot;
mod plugin;
pub mod slug;
pub mod stats_overlay;

pub use plugin::ShadersPlugin;

include!(concat!(env!("OUT_DIR"), "/shader_handles.rs"));
