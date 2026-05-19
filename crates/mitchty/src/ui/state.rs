/// Which rendering backend is currently active.
///
/// `Egui` is the only real option right now. `Feathers` is a placeholder that
/// will abuse bevy ui/feathers more once that is ready post 0.19.x. The intent
/// is that flipping this in `UiState` at runtime and gating systems with
/// `run_if_enabled` / `run_if_disabled` is a live migration path so I can
/// migrate stuff bit by bit and then do a wholesale swap and rip out the whole
/// backend stuff entirely with egui too later.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum UiBackend {
    #[default]
    Egui,
    /// For the future! (tm)(c)(r)
    #[expect(
        dead_code,
        reason = "placeholder for a real future feathers/BSN backend when that can happen until then suck it future mitch"
    )]
    Feathers,
}

/// Global control-center state for the UI layer.
///
/// This is intentionally backend-agnostic it describes *what* the UI should
/// be doing, not *how* a particular backend renders it.  Each backend's plugin
/// reads this resource and responds accordingly. Least thats my plan stan.
///
/// Currently only `Egui` has anything useful, I tried out bevy ui and some of
/// the other options and I'll just abuse egui for now.
#[derive(bevy::prelude::Resource)]
pub struct UiState {
    /// Which rendering backend is active `Egui` only later
    pub backend: UiBackend,

    /// Whether the top menu bar or whatever the hell its feathers equivalent be
    /// should be visible.
    pub menu_bar_visible: bool,

    /// Master kill switch when `false` no UI rendering runs at all. I have no
    /// idea where I'll use this yet but I should stop here hold my beer at
    /// times.
    pub enabled: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            backend: UiBackend::default(),
            menu_bar_visible: true,
            enabled: true,
        }
    }
}

/// Which side of the menu bar a `UiPanel` entry is anchored to.
///
/// `Left`  = rendered left-to-right in normal flow order.
/// `Right` = rendered right-to-left note lower `order` values end up further
/// right on screen.
///
/// Aka 0 10 20 ..... 20 10 0 = Left .... Right and the `order` values control
/// whereabouts in the menu bar this all fits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MenuAnchor {
    #[default]
    Left,
    Right,
}

/// Run condition that returns `true` when `UiState::backend` is `Egui`.
///
/// Will be abusing this with `configure_sets` to gate egui-specific system sets
/// at runtime rather than at compile time, so I can have the debug menu help me
/// control which ui is used at runtime.
///
/// ```rust,ignore
/// app.configure_sets(
///     EguiPrimaryContextPass,
///     EguiSystems.run_if(egui_backend_active()),
/// );
/// ```
pub fn egui_backend_active() -> impl FnMut(Option<bevy::prelude::Res<UiState>>) -> bool + Clone {
    move |res: Option<bevy::prelude::Res<UiState>>| {
        res.map(|s| s.backend == UiBackend::Egui).unwrap_or(true)
    }
}

/// ECS identity for any plugin that represents a piece of gooey.
///
/// Spawned as a component for now in preparation for bsn future stuff. The
/// stable `id` field is what future bsn scenes and feathers templates will
/// target when the backend changes at runtime. This entity and its `UiPanel`
/// component survive a backend swap untouched so that both backends use the
/// same source data internally.
#[derive(bevy::prelude::Component)]
pub struct UiPanel {
    /// Stable identifier, e.g. `"about"`, `"reveries"`. Must be unique.
    pub id: &'static str,
    /// Which side of the menu bar this entry belongs to.
    pub anchor: MenuAnchor,
    /// Sort priority within the anchor group.
    pub order: i32,
}
