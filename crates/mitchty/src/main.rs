mod ai;
mod assets;
mod mesh_effect;
mod plugins;
mod post_process;
mod ui;

pub use mitchty::CameraMode;

use plugins::camera::CameraPlugin;
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
    // TODO: I added platform_startup to hide all the wonky wine/windows hacks,
    // maybe the panic stuff for wasm fits too? Its too late now I'll deal with
    // that later I only did the easy fruit the web one I need to sleep on a bit.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    lib::quirks::platform_startup();

    #[cfg(not(target_arch = "wasm32"))]
    let (enable_gamepad, ui_config) = plugins::cli::parse_native_args();

    #[cfg(target_arch = "wasm32")]
    let enable_gamepad = false;

    #[cfg(target_arch = "wasm32")]
    let ui_config = plugins::cli::parse_wasm_args();

    let mut app = App::new();

    app.init_resource::<PluginRegistry>()
        .add_systems(PreUpdate, sync_registry_to_plugins);

    app.add_plugins(assets::create_default_plugins(enable_gamepad))
        .add_plugins(assets::AssetConfigPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(ShadersPlugin)
        .add_plugins(PostProcessPlugin)
        .add_plugins(TransformGizmoPlugin)
        .add_plugins(MeshEffectPlugin)
        .add_plugins(bevy_pretty_text::prelude::PrettyTextPlugin)
        .add_plugins(flan::PlotPlugin)
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

    app.add_plugins((CameraPlugin, FullscreenEffectPlugin))
        .add_plugins(FpsPlugin)
        .add_plugins(ScenePlugin)
        .add_plugins(Text3dPlugin)
        .add_plugins(HuePlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(SettingsUiPlugin)
        .add_plugins(ReveriesPlugin)
        .add_plugins(TerminalPlugin);

    app.run();
}
