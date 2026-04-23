//!
//! Theme related Bevy plugin `ThemePlugin`, for now only really handles
//! light/dark setup. More in future once I start doing more theme related.
//!
//! Owns platform-specific detection crap and fires `ThemeChanged` messages for
//! Bevy whenever the theme mode differs from the last known setting. Egui
//! specific systems or anything else can read those messages and react
//! independently.

// Wasm js bridge types.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;

#[cfg(feature = "egui")]
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use bevy::prelude::*;

use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};
use crate::ui::config::{ThemeChoice, UiConfig};

/// `SystemSet` that `apply_theme_change` is ordered to run after.
///
/// All consumers should put themselves in this set so they run correctly after a theme message happens.
///
/// Only present when the `egui` feature is enabled.
#[cfg(feature = "egui")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct EguiThemeSet;

/// `SystemSet` that gates all per-frame systems owned by [`ThemePlugin`].
///
/// Controlled by `PluginEnabled::<ThemePlugin>`. Disabling stops OS/browser
/// theme-change polling and egui visual application. Note: need to brain up
/// what else disabling this should do as I want turning theming off to do
/// something else but no idea what that is yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct ThemeSystems;

/// Bevy message fired when the OS or browser dark/light preference changes.
///
/// On native, emitted by [`poll_system_theme`] every second at best.
/// On WASM, emitted by [`drain_wasm_theme_events`] when the `matchMedia`
/// listener fires.
#[derive(Debug, Clone, Copy)]
pub struct ThemeChanged(pub dark_light::Mode);
impl bevy::ecs::message::Message for ThemeChanged {}

/// Holds the last os related theme value seen by [`poll_system_theme`].
///
/// Initialized at `Default` so the very first poll has a sane default to
/// diff against. Falls back to `Dark` when `dark_light::detect()` fails.
///
/// Native-only; WASM uses the `matchMedia` listener instead.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct LastKnownTheme(pub dark_light::Mode);

#[cfg(not(target_arch = "wasm32"))]
impl Default for LastKnownTheme {
    fn default() -> Self {
        Self(dark_light::detect().unwrap_or(dark_light::Mode::Dark))
    }
}

/// Keeps the `matchMedia` closure and `MediaQueryList` alive for the whole app
/// lifetime. Both must stay alive or the listener silently unregisters.
///
/// Stored as a `NonSend` resource so it doesn't need to be `Send` for the js
/// bridge.
#[cfg(target_arch = "wasm32")]
pub struct WasmThemeListener {
    /// The wasm_bindgen closure listener.
    _closure: Closure<dyn FnMut(web_sys::MediaQueryListEvent)>,
    /// The MediaQueryList the listener is attached to.
    _mql: web_sys::MediaQueryList,
}

/// Shared slot written by the JS closure and drained by `drain_wasm_theme_events`.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
struct WasmThemePending(std::sync::Arc<std::sync::Mutex<Option<dark_light::Mode>>>);

/// Bevy plugin that detects OS or browser dark-light preferences and emits
/// [`ThemeChanged`] messages to correspond with them.
pub struct ThemePlugin;

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PluginEnabled::<ThemePlugin>::default())
            .configure_sets(Update, ThemeSystems.run_if(run_if_enabled::<ThemePlugin>()))
            .add_message::<ThemeChanged>();

        #[cfg(not(target_arch = "wasm32"))]
        app.init_resource::<LastKnownTheme>().add_systems(
            Update,
            poll_system_theme
                .run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                ))
                .in_set(ThemeSystems),
        );

        #[cfg(target_arch = "wasm32")]
        app.add_systems(Startup, setup_wasm_theme_listener)
            .add_systems(Update, drain_wasm_theme_events.in_set(ThemeSystems));

        // EguiThemeSet configure_sets is an ordering anchor used by other
        // plugins for now.
        #[cfg(feature = "egui")]
        app.configure_sets(EguiPrimaryContextPass, EguiThemeSet)
            .add_systems(
                EguiPrimaryContextPass,
                apply_theme_change.after(EguiThemeSet).in_set(ThemeSystems),
            );

        if let Some(mut registry) = app.world_mut().get_resource_mut::<PluginRegistry>() {
            registry.register::<ThemePlugin>("Theme", true);
        }
    }
}

/// Native system to poll `dark_light::detect()` every second to detect dynamic
/// theme changes.
///
/// Emits `ThemeChanged` messages only when the detected mode differs from the last
/// seen theme value and the user has not explicitly chosen a theme via [`UiConfig`].
#[cfg(not(target_arch = "wasm32"))]
fn poll_system_theme(
    ui_config: Res<UiConfig>,
    mut last: ResMut<LastKnownTheme>,
    mut events: bevy::ecs::message::MessageWriter<ThemeChanged>,
) {
    // User choice wins - nothing to do.
    if ui_config.theme != ThemeChoice::Auto {
        return;
    }

    let current = dark_light::detect().unwrap_or(dark_light::Mode::Dark);
    if current != last.0 {
        last.0 = current;
        events.write(ThemeChanged(current));
    }
}

/// WASM system to register a `matchMedia` change listener at app startup.
///
/// The listener writes the new mode from the js bridge into a shared
/// `Arc<Mutex<Option<...>>>`. A separate per-frame system
/// `drain_wasm_theme_events` drains that slot into the Bevy message bus. Both
/// the `Closure` and `MediaQueryList` are stored as a `NonSend` resource so
/// they stay alive for the app's lifetime.
#[cfg(target_arch = "wasm32")]
fn setup_wasm_theme_listener(world: &mut World) {
    use std::sync::{Arc, Mutex};

    let pending: Arc<Mutex<Option<dark_light::Mode>>> = Arc::new(Mutex::new(None));
    let pending_clone = pending.clone();

    // TODO: I have no idea wat to do on failures here. I barely understand wasm
    // as it is js interop is veritable black magique.
    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            bevy::log::warn!("setup_wasm_theme_listener: no window object, skipping");
            return;
        }
    };

    let mql = match window.match_media("(prefers-color-scheme: dark)") {
        Ok(Some(mql)) => mql,
        _ => {
            bevy::log::warn!(
                "setup_wasm_theme_listener: matchMedia not available, skipping listener"
            );
            return;
        }
    };

    let closure: Closure<dyn FnMut(web_sys::MediaQueryListEvent)> =
        Closure::wrap(Box::new(move |e: web_sys::MediaQueryListEvent| {
            let mode = if e.matches() {
                dark_light::Mode::Dark
            } else {
                dark_light::Mode::Light
            };
            if let Ok(mut slot) = pending_clone.lock() {
                *slot = Some(mode);
            }
        }));

    if let Err(err) =
        mql.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
    {
        bevy::log::warn!(
            "setup_wasm_theme_listener: failed to add event listener: {:?}",
            err
        );
        return;
    }

    // Store as NonSend so the closure and mql stay alive.
    world.insert_non_send_resource(WasmThemeListener {
        _closure: closure,
        _mql: mql,
    });

    // Store the Arc so the drain system can read it without unsafe.
    world.insert_resource(WasmThemePending(pending));
}

/// WASM system to drain the shared pending slot into a [`ThemeChanged`] bevy
/// message.
///
/// Skipped when the user has an explicit theme choice to be consistent with the
/// native [`poll_system_theme`] behavior.
#[cfg(target_arch = "wasm32")]
fn drain_wasm_theme_events(
    ui_config: Res<UiConfig>,
    pending_res: Option<Res<WasmThemePending>>,
    mut events: bevy::ecs::message::MessageWriter<ThemeChanged>,
) {
    // User choice wins - nothing to do.
    if ui_config.theme != ThemeChoice::Auto {
        return;
    }

    let Some(pending_res) = pending_res else {
        return;
    };

    let Ok(mut slot) = pending_res.0.lock() else {
        return;
    };

    if let Some(mode) = slot.take() {
        events.write(ThemeChanged(mode));
    }
}

/// Apply an incoming [`ThemeChanged`] message to egui visuals.
///
/// Skipped entirely when the user has set an explicit [`ThemeChoice`].
#[cfg(feature = "egui")]
fn apply_theme_change(
    mut contexts: EguiContexts,
    ui_config: Res<UiConfig>,
    mut events: bevy::ecs::message::MessageReader<ThemeChanged>,
) -> Result {
    // User picked something, drain the queue and bail.
    if ui_config.theme != ThemeChoice::Auto {
        for _ in events.read() {}
        return Ok(());
    }

    // Consume all pending messages last one wins. Only one should arrive at
    // once but be defensive about it just in case.
    let mut new_visuals: Option<egui::Visuals> = None;
    for ThemeChanged(mode) in events.read() {
        new_visuals = Some(match mode {
            dark_light::Mode::Dark => egui::Visuals::dark(),
            dark_light::Mode::Light | dark_light::Mode::Unspecified => egui::Visuals::light(),
        });
    }

    if let Some(visuals) = new_visuals {
        let ctx = contexts.ctx_mut()?;
        ctx.set_visuals(visuals);
    }

    Ok(())
}

// TODO: In porting things over noticed that there is duplication that likely
// should be DRY not WET
/// Resolve which egui [`Visuals`][egui::Visuals] to apply at startup.
///
/// Resolution order (first match wins):
/// 1. User's explicit [`ThemeChoice::Dark`] or [`ThemeChoice::Light`].
/// 2. OS / browser preference via `dark_light::detect()`.
/// 3. Time-of-day heuristic: 07:00–17:59 -> Light, otherwise -> Dark.
/// 4. Hard fallback: Dark.
#[cfg(feature = "egui")]
pub fn resolve_initial_theme(choice: ThemeChoice) -> egui::Visuals {
    match choice {
        ThemeChoice::Dark => return egui::Visuals::dark(),
        ThemeChoice::Light => return egui::Visuals::light(),
        ThemeChoice::Auto => {}
    }

    match dark_light::detect().unwrap_or(dark_light::Mode::Unspecified) {
        dark_light::Mode::Dark => return egui::Visuals::dark(),
        dark_light::Mode::Light => return egui::Visuals::light(),
        dark_light::Mode::Unspecified => {}
    }

    // Hold my beer and use time of day to SWAG a guesstimate.
    let hour = jiff::Zoned::now().hour();
    if (7..18).contains(&hour) {
        return egui::Visuals::light();
    }

    // If we ever get here just go for Dark, its like the Rock of Roshambo for
    // themes.
    egui::Visuals::dark()
}

/// Map `ThemeChoice` to the default background `bevy::color::Srgba` color.
///
/// Dark -> black, Light -> white. `Auto` resolves to whatever the correct color
/// should be via `resolve_initial_theme`.
#[cfg(feature = "egui")]
pub fn theme_default_color(choice: ThemeChoice) -> bevy::color::Srgba {
    let is_dark = match choice {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::Auto => resolve_initial_theme(ThemeChoice::Auto).dark_mode,
    };
    if is_dark {
        bevy::color::Srgba::new(0.0, 0.0, 0.0, 1.0)
    } else {
        bevy::color::Srgba::new(1.0, 1.0, 1.0, 1.0)
    }
}
