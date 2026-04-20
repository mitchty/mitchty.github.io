use crate::ui::world_clock::{SortColumn, SortDir};
use jiff::Timestamp;

/// Auto chains as follows:
///   - OS/Browser reports the "right" theme
///   - Between 7-18 light, otherwise dark
///   - If that fails somehow dark I guess.
///
/// Non Auto enum means just use that as the user picked it don't change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    Auto,
    /// Always use dark.
    Dark,
    /// Always use light.
    Light,
}

/// Identifies a toggleable UI window by name for arg parsing inputs on what to
/// show at startup from clap or query params in wasm/web.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiWindow {
    /// The world clock app
    WorldClock,

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
            "world-clock" | "world_clock" => Some(UiWindow::WorldClock),
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

    /// Open the World Clock window on startup default: `false`.
    pub show_world_clock: bool,

    /// Open the Recognizer window on startup default: `false`.
    pub show_recognizer: bool,

    /// Open the Data Viewer window on startup default: `false`, native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub show_data_viewer: bool,

    /// Open a specific reverie by name on startup.
    ///
    /// Stores the raw string from `--reverie` CLI/native or `?reverie=`
    /// URL/wasm. Resolved once in `setup_egui` (for now...) by matching against
    /// `ReverieKey` on the spawned reverie entities. Supports canonical keys
    /// `"some_ns/foo"`, aliases, and display names all matched
    /// case-insensitively to match uri insensitivity.
    pub initial_reverie: Option<String>,

    /// Override the initial timezone list shown in World Clock. When non-empty
    /// these replace the hardcoded defaults. Each entry is an IANA tz name.
    pub initial_timezones: Vec<String>,

    /// Pre-seeded alarms for World Clock. Each entry is a tuple of
    /// `(utc_timestamp, iana_tz, optional_label)`. The timestamp is the exact
    /// UTC moment the alarm fires; tz is for display; label is the optional
    /// human-readable name shown in the 3D countdown text.
    pub initial_alarms: Vec<(Timestamp, String, Option<String>)>,

    /// Initial sort column for the World Clock table. `None` means insertion order.
    pub initial_sort_col: SortColumn,

    /// Initial sort direction for the World Clock table.
    pub initial_sort_dir: SortDir,

    /// If `Some`, the World Clock starts with the clock frozen at this UTC moment
    /// instead of showing live time.
    pub initial_pinned: Option<Timestamp>,

    /// Which egui visual theme to apply on startup.
    pub theme: ThemeChoice,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_menu_bar: true,
            show_world_clock: false,
            show_recognizer: false,
            #[cfg(not(target_arch = "wasm32"))]
            show_data_viewer: false,
            initial_reverie: None::<String>,
            initial_timezones: Vec::new(),
            initial_alarms: Vec::new(),
            initial_sort_col: SortColumn::None,
            initial_sort_dir: SortDir::Asc,
            initial_pinned: None,
            theme: ThemeChoice::Auto,
        }
    }
}

impl UiConfig {
    pub fn enable_window(&mut self, window: UiWindow) {
        match window {
            UiWindow::WorldClock => self.show_world_clock = true,
            UiWindow::Recognizer => self.show_recognizer = true,
            #[cfg(not(target_arch = "wasm32"))]
            UiWindow::DataViewer => self.show_data_viewer = true,
        }
    }
}
