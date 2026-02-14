#[cfg(feature = "egui")]
mod egui;
#[cfg(feature = "egui")]
pub use egui::*;

#[cfg(feature = "feathers")]
mod feathers;
#[cfg(feature = "feathers")]
pub use feathers::*;

#[cfg(not(any(feature = "egui", feature = "feathers")))]
compile_error!("this is a gooey only app, need feathers or egui");

#[cfg(all(feature = "egui", feature = "feathers"))]
compile_error!("this gooey app doesn't support egui and feathers atm");
