pub mod config;
pub use config::*;

#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
pub mod data_viewer;
#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
#[allow(unused_imports)]
pub use data_viewer::*;

#[cfg(feature = "egui")]
mod egui;
#[cfg(feature = "egui")]
pub use egui::*;

#[cfg(feature = "egui")]
pub mod recognizer;
#[cfg(feature = "egui")]
#[allow(unused_imports)]
pub use recognizer::*;

#[cfg(feature = "feathers")]
mod feathers;
#[cfg(feature = "feathers")]
pub use feathers::*;

#[cfg(feature = "egui")]
pub mod world_clock;
#[cfg(feature = "egui")]
#[allow(unused_imports)]
pub use world_clock::*;

#[cfg(feature = "egui")]
pub mod losant;
#[cfg(feature = "egui")]
#[allow(unused_imports)]
pub use losant::*;

#[cfg(not(any(feature = "egui", feature = "feathers")))]
compile_error!("this is a gooey only app, need feathers or egui");

#[cfg(all(feature = "egui", feature = "feathers"))]
compile_error!("this gooey app doesn't support egui and feathers atm");
