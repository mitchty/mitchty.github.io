// For profiling memory issues
#[cfg(all(feature = "dhat-heap", feature = "stats-alloc"))]
compile_error!("dhat-heap and stats-alloc are mutually exclusive: enable at most one");
#[cfg(all(feature = "dhat-heap", feature = "jemalloc-pprof"))]
compile_error!("dhat-heap and jemalloc-pprof are mutually exclusive: enable at most one");
#[cfg(all(feature = "stats-alloc", feature = "jemalloc-pprof"))]
compile_error!("stats-alloc and jemalloc-pprof are mutually exclusive: enable at most one");

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static DHAT_ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "stats-alloc")]
#[global_allocator]
pub static ALLOC_STATS: &stats_alloc::StatsAlloc<std::alloc::System> =
    &stats_alloc::INSTRUMENTED_SYSTEM;

#[cfg(all(
    feature = "jemalloc-pprof",
    not(target_arch = "wasm32"),
    not(target_env = "msvc")
))]
#[global_allocator]
static JEMALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod ai;
mod assets;
mod mesh_effect;
mod plugins;
mod post_process;
mod profiling;
mod ui;

pub use mitchty::CameraMode;

use plugins::camera::CameraPlugin;
use plugins::fonts::FontsPlugin;
use plugins::fps::FpsPlugin;
use plugins::fullscreen::FullscreenEffectPlugin;
use plugins::help::HelpPlugin;
use plugins::hue::HuePlugin;
use plugins::input::InputPlugin;
use plugins::reveries::ReveriesPlugin;
use plugins::scene::ScenePlugin;

use plugins::terminal::TerminalPlugin;
use plugins::text3d::Text3dPlugin;
use plugins::{PluginRegistry, sync_registry_to_plugins};

use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
#[cfg(feature = "feathers")]
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::prelude::*;
use flan::shaders::ShadersPlugin;
use mesh_effect::MeshEffectPlugin;
use post_process::PostProcessPlugin;
use transform_gizmo_bevy::prelude::*;
use ui::SettingsUiPlugin;

fn main() {
    // dhat profiler guard that must be the very first local so it is dropped last,
    // flushing dhat-heap.json after everything else has been cleaned up.
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    // TODO: I added platform_startup to hide all the wonky wine/windows hacks,
    // maybe the panic stuff for wasm fits too? Its too late now I'll deal with
    // that later I only did the easy fruit the web one I need to sleep on a bit.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    lib::quirks::platform_startup();

    #[cfg(not(target_arch = "wasm32"))]
    let (enable_gamepad, ui_config, without_plugins) = plugins::cli::parse_native_args();

    #[cfg(target_arch = "wasm32")]
    let enable_gamepad = false;

    #[cfg(target_arch = "wasm32")]
    let ui_config = plugins::cli::parse_wasm_args();

    // wasm is always a release build so debug_assertions is never set;
    // without_plugins declared here only so the let _ = below compiles.
    #[cfg(target_arch = "wasm32")]
    let without_plugins: Vec<String> = Vec::new();

    let mut app = App::new();

    // Insert DisabledPlugins before any plugin's build() runs so they can
    // read it via bavy::disabled::is_disabled(app.world(), "name").
    // Only exists in debug builds; release always sees is_disabled() == false.
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    {
        let set = without_plugins
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        app.insert_resource(plugins::disabled::DisabledPlugins(set));
    }

    // Suppress unused-variable warning in release / wasm builds.
    #[cfg(any(not(debug_assertions), target_arch = "wasm32"))]
    let _ = without_plugins;

    app.init_resource::<PluginRegistry>()
        .add_systems(PreUpdate, sync_registry_to_plugins);

    app.add_plugins(assets::create_default_plugins(enable_gamepad))
        .add_plugins(assets::AssetConfigPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(ShadersPlugin);

    if !bavy::disabled::is_disabled(app.world(), "postprocess") {
        app.add_plugins(PostProcessPlugin);
    } else {
        info!("PostProcessPlugin disabled via --without-plugins");
    }

    app.add_plugins(TransformGizmoPlugin)
        .add_plugins(MeshEffectPlugin)
        .add_plugins(bevy_pretty_text::prelude::PrettyTextPlugin)
        .insert_resource(ui_config);

    #[cfg(feature = "egui")]
    {
        app.add_plugins(bevy_egui::EguiPlugin::default());
    }

    #[cfg(feature = "feathers")]
    {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(sys::SysPlugin);

    app.add_plugins((CameraPlugin, FullscreenEffectPlugin))
        .add_plugins(FontsPlugin)
        .add_plugins(FpsPlugin)
        .add_plugins(ScenePlugin)
        .add_plugins(Text3dPlugin)
        .add_plugins(HuePlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(SettingsUiPlugin)
        .add_plugins(ReveriesPlugin)
        .add_plugins(TerminalPlugin);

    app.add_plugins(profiling::ProfilingPlugin);

    #[cfg(feature = "debug")]
    app.add_plugins(bevy::diagnostic::SystemInformationDiagnosticsPlugin)
        .add_plugins(bevy::diagnostic::LogDiagnosticsPlugin::default());

    #[cfg(feature = "debug")]
    app.add_systems(
        Update,
        debug_asset_counts.run_if(bevy::time::common_conditions::on_timer(
            std::time::Duration::from_secs(1),
        )),
    );

    app.run();
}

#[cfg(feature = "debug")]
fn debug_asset_counts(images: Res<Assets<Image>>, meshes: Res<Assets<Mesh>>) {
    info!("images: {}, meshes: {}", images.len(), meshes.len());
}
