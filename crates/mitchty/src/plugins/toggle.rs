use bevy::prelude::*;
use std::marker::PhantomData;

/// A per-plugin enabled flag that gates an entire `SystemSet` via
/// `run_if_enabled::<T>()`.
///
/// Inserted by default as `true` in each relevant plugin's `build()`. Flip it
/// to `false` at any point during the run to stop all of that plugin's gated
/// systems from being scheduled.
/// ```
#[derive(Resource)]
pub struct PluginEnabled<T: 'static + Send + Sync> {
    pub enabled: bool,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static + Send + Sync> Default for PluginEnabled<T> {
    fn default() -> Self {
        Self {
            enabled: true,
            _marker: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<T: 'static + Send + Sync> PluginEnabled<T> {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            _marker: PhantomData,
        }
    }

    /// Shorthand to construct the disabled state.
    pub fn disabled() -> Self {
        Self::new(false)
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Run condition for a plugin's `SystemSet`.
///
/// Returns `true` when `PluginEnabled<T>` either doesn't exist so that plugins
/// that haven't opted in to this nonsense are unaffected or has `enabled ==
/// true` explicitly.
///
/// Use this with `configure_sets` like so:
///
/// ```rust,ignore
/// app.configure_sets(
///     Update,
///     MyPluginSystems.run_if(run_if_enabled::<MyPlugin>()),
/// );
/// ```
pub fn run_if_enabled<T: 'static + Send + Sync>()
-> impl FnMut(Option<Res<PluginEnabled<T>>>) -> bool + Clone {
    move |res: Option<Res<PluginEnabled<T>>>| res.map(|r| r.enabled).unwrap_or(true)
}

/// Run condition that is the complement of [`run_if_enabled`] that returns `true`
/// only when `PluginEnabled<T>` exists **and** is disabled.
///
/// The primary use case is incrementally porting a feature from one UI backend
/// to another without ever breaking the running app. For example, while porting
/// the World Clock from egui to native Bevy UI:
///
/// ```rust,ignore
/// // Existing egui implementation if toggle is enabled.
/// app.configure_sets(
///     EguiPrimaryContextPass,
///     WorldClockEguiSystems.run_if(run_if_enabled::<WorldClockEguiPlugin>()),
/// );
///
/// // New Bevy UI implementation runs only while the egui plugin is off.
/// // Will let me flip one checkbox in the debug menu and the swap is live instantly.
/// app.configure_sets(
///     Update,
///     WorldClockBevySystems.run_if(run_if_disabled::<WorldClockEguiPlugin>()),
/// );
/// ```
///
/// Returns `false` when the resource is absent, unlike `run_if_enabled` which
/// defaults to `true` when absent.
#[allow(dead_code)]
pub fn run_if_disabled<T: 'static + Send + Sync>()
-> impl FnMut(Option<Res<PluginEnabled<T>>>) -> bool + Clone {
    move |res: Option<Res<PluginEnabled<T>>>| res.map(|r| !r.enabled).unwrap_or(false)
}

/// Function pointer type used to propagate a `PluginRegistry` entry's `enabled`
/// flag into the corresponding `PluginEnabled<T>` resource.
///
/// Stored as a plain `fn` and not a closure so `PluginRegistryEntry` stays
/// `Send + Sync` without needing a `Box<dyn Fn>`.
type SyncFn = fn(&mut World, bool);

/// A single entry in `PluginRegistry`.
pub struct PluginRegistryEntry {
    /// Human-readable name shown in the debug plugin menu/future ui.
    pub name: &'static str,
    /// Current toggle state. The `sync_registry_to_plugins` exclusive system
    /// propagates this into `PluginEnabled<T>` every `PreUpdate`.
    pub enabled: bool,
    sync: SyncFn,
}

/// Global registry of all plugins that opt into runtime toggling.
///
/// Plugins call `registry.register::<Self>("Display Name", true)` inside their
/// `Plugin::build()` after `PluginRegistry` has been initialised in `main()`.
/// The egui debug menu reads and mutates `entries` directly; changes are
/// flushed to the real `PluginEnabled<T>` resources by
/// `sync_registry_to_plugins` each `PreUpdate`.
#[derive(Resource, Default)]
pub struct PluginRegistry {
    pub entries: Vec<PluginRegistryEntry>,
}

impl PluginRegistry {
    /// Register a plugin so it appears in the debug menu.
    ///
    /// `name` is the display label. `default_enabled` sets the initial Update state
    ///
    /// The sync function is monomorphized here so each `T` gets its own
    /// `fn(&mut World, bool)` compiled in and no heap allocation is needed.
    pub fn register<T: 'static + Send + Sync>(
        &mut self,
        name: &'static str,
        default_enabled: bool,
    ) {
        self.entries.push(PluginRegistryEntry {
            name,
            enabled: default_enabled,
            sync: |world, val| {
                if let Some(mut r) = world.get_resource_mut::<PluginEnabled<T>>() {
                    r.enabled = val;
                }
            },
        });
    }
}

/// Exclusive system that runs in `PreUpdate` that propagates every
/// `PluginRegistry` entry's `enabled` flag into its `PluginEnabled<T>` resource
/// so `run_if_enabled` conditions see the update on the same frame's `Update`
/// schedule.
///
/// Collects `(enabled, sync_fn)` pairs first to avoid holding a shared borrow
/// on `World` while calling the mutable sync functions.
pub fn sync_registry_to_plugins(world: &mut World) {
    let pairs: Vec<(bool, SyncFn)> = world
        .get_resource::<PluginRegistry>()
        .map(|r| r.entries.iter().map(|e| (e.enabled, e.sync)).collect())
        .unwrap_or_default();

    for (enabled, sync) in pairs {
        sync(world, enabled);
    }
}
