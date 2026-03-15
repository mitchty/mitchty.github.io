/// Identifies a toggleable UI window by name for arg parsing inputs on what to
/// show at startup from clap or query params in wasm/web.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiWindow {
    /// The Japanese character recognizer
    Recognizer,

    /// The NPZ data viewer hack
    #[cfg(not(target_arch = "wasm32"))]
    DataViewer,
}

impl UiWindow {
    /// Parse a window name from a string slug; case-insensitive.
    ///
    /// Returns `None` for unknown names that can't be found at runtime.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "recognizer" => Some(UiWindow::Recognizer),
            #[cfg(not(target_arch = "wasm32"))]
            "data-viewer" | "data_viewer" => Some(UiWindow::DataViewer),
            _ => None,
        }
    }
}

#[derive(bevy::prelude::Resource, Debug, Clone)]
pub struct UiConfig {
    /// Show the egui menu bar on startup default: `true`.
    pub show_menu_bar: bool,

    /// Open the Recognizer window on startup default: `false`.
    pub show_recognizer: bool,

    /// Open the Data Viewer window on startup default: `false`, native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub show_data_viewer: bool,

    /// Open a specific post by index on startup, normally not done, opt-in only
    /// or via links for the web version.
    ///
    /// Resolves from the name/string passed in via `post_index_for_name`. Only
    /// read once and mapped to an index to seed the `ActivePost`. Not used at
    /// runtime afterwards. One shot system struct.
    pub initial_post: Option<usize>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_menu_bar: true,
            show_recognizer: false,
            #[cfg(not(target_arch = "wasm32"))]
            show_data_viewer: false,
            initial_post: None,
        }
    }
}

impl UiConfig {
    pub fn enable_window(&mut self, window: UiWindow) {
        match window {
            UiWindow::Recognizer => self.show_recognizer = true,
            #[cfg(not(target_arch = "wasm32"))]
            UiWindow::DataViewer => self.show_data_viewer = true,
        }
    }
}
