use bevy::prelude::*;

#[cfg(not(debug_assertions))]
use bevy::asset::embedded_asset;

use bevy::asset::io::web::WebAssetPlugin;

/// Determine the base asset path for the AssetPlugin in debug builds.
/// Returns the path based on BEVY_ASSET_PATH env var or a fallback path.
///
/// Here to make testing with and without embedding easier.
///
/// Points to workspace root to allow access to assets in other crates
#[allow(dead_code)]
pub fn get_asset_base_path(bevy_asset_path_env: Option<String>, manifest_dir: &str) -> String {
    use std::path::PathBuf;

    // Finds workspace root for the asset_base_path
    bevy_asset_path_env.unwrap_or_else(|| {
        PathBuf::from(manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf()
            .to_string_lossy()
            .to_string()
    })
}

// Dead code allowed here cause its unit tested in both release/dev profiles but
// only ever used in one. Not a huge deal to have an extra function that isn't
// used in both profiles in the binary..

/// Get asset path for debug builds from fs or http if wasm
/// Prepends crates/mitchty/src/assets/ since asset base is workspace root now
#[allow(dead_code)]
pub fn asset_path_debug(path: &str) -> String {
    format!("crates/mitchty/src/assets/{}", path)
}

/// Get asset path for release builds, always embedded assets for release builds
#[allow(dead_code)]
pub fn asset_path_release(path: &str) -> String {
    format!("embedded://mitchty/assets/{}", path)
}

/// Trampoline to asset_path_debub/release depending on build profile.
pub fn asset_path(path: &str) -> String {
    #[cfg(debug_assertions)]
    {
        asset_path_debug(path)
    }
    #[cfg(not(debug_assertions))]
    {
        asset_path_release(path)
    }
}

/// Macro to generate asset path expressions based on build profile
/// Use this when you need a compile-time &'static str (e.g., for ShaderRef::from(&str))
///
/// # Examples
/// ```
/// use crate::asset_path_raw;
/// let shader_ref: ShaderRef = asset_path_raw!("shaders/my_shader.wgsl").into();
/// ```
#[macro_export]
macro_rules! asset_path_raw {
    ($path:expr) => {
        if cfg!(debug_assertions) {
            concat!("crates/mitchty/src/assets/", $path)
        } else {
            concat!("embedded://mitchty/assets/", $path)
        }
    };
}

/// Plugin that configures assets based on build type and platform
pub struct AssetConfigPlugin;

impl Plugin for AssetConfigPlugin {
    fn build(&self, _app: &mut App) {
        // Only embed assets in release builds
        #[cfg(not(debug_assertions))]
        {
            // Environment maps for the cube hues
            embedded_asset!(
                _app,
                "assets/environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2"
            );
            embedded_asset!(
                _app,
                "assets/environment_maps/pisa_specular_rgb9e5_zstd.ktx2"
            );

            // Fonts for the 3d text thingy. I should make a 3d console/shell.... maybe?
            embedded_asset!(_app, "assets/fonts/FiraMono-Medium.ttf");
            // Japanese font used by the egui Recognizer sidebar.
            embedded_asset!(_app, "assets/fonts/NotoSansJP-Regular.ttf");

            // gltf model for default abuse loading
            embedded_asset!(_app, "assets/mitchty.glb");
        }
    }
}

// TODO: I might need to find out if for native I can use embedded reloading or
// not, saw it in the feature list. Probably not a huge thing.
/// Build the default Bevy plugin group for the current platform.
///
/// Note gamepad spiel is to make running windows binaries in wine more easily.
///
/// Nothing yet.... in this supports it. More a wine bug when a gamepad is
/// present as a usb device.
pub fn create_default_plugins(enable_gamepad: bool) -> bevy::app::PluginGroupBuilder {
    // Need to register the http(s) plugin asset server before the DefaultPlugins.
    let web_asset_plugin = WebAssetPlugin {
        silence_startup_warning: true,
    };

    // In debug builds, configure AssetPlugin to auto reload from the
    // filesystem. UnapprovedPathMode::Allow lets absolute paths pass through
    // FileAssetReader without being blocked thats kinda the point for this app.
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    let plugins = {
        use bevy::asset::{AssetPlugin, UnapprovedPathMode};

        let asset_base = get_asset_base_path(
            std::env::var("BEVY_ASSET_PATH").ok(),
            env!("CARGO_MANIFEST_DIR"),
        );

        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_base,
                unapproved_path_mode: UnapprovedPathMode::Allow,
                ..default()
            })
            .add_before::<AssetPlugin>(web_asset_plugin)
    };

    // WASM-specific configuration match the window and canvas size and register
    // remote source asset loading stuff.
    #[cfg(target_arch = "wasm32")]
    let plugins = {
        use bevy::asset::AssetPlugin;
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    fit_canvas_to_parent: true,
                    // Ok so we *have* to leave default event handling off.
                    // Bevy's setup here is all or nothing, so if I *want* pinch
                    // to zoom, which I do, I can't ignore all events as
                    // copy/paste as an example are in that category as well.
                    // Soooo apparently we need to abuse the javascript bridge
                    // to allow those to yeet in from the outside world.
                    //
                    // I swear the web is the most inconsistent platform in the
                    // world. Why anyone likes to develop for it is beyond me.
                    // This just reinforces my desire to never learn js/web
                    // programming.
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            })
            .add_before::<AssetPlugin>(web_asset_plugin)
    };

    // Release native builds embedded assets are registered via
    // AssetConfigPlugin, but FileAssetReader is also setup so absolute local
    // paths work too. UnapprovedPathMode::Allow is needed for crap that starts
    // with a / or I think like C:\ on windows but not sure on that.
    #[cfg(all(not(debug_assertions), not(target_arch = "wasm32")))]
    let plugins = {
        use bevy::asset::{AssetPlugin, UnapprovedPathMode};
        DefaultPlugins
            .set(AssetPlugin {
                unapproved_path_mode: UnapprovedPathMode::Allow,
                ..default()
            })
            .add_before::<AssetPlugin>(web_asset_plugin)
    };

    // Need --with-gamepad anywhere for now for bevy_input to care about
    // gamepads. Future me problem for when/if I add support.
    if enable_gamepad {
        plugins
    } else {
        plugins.disable::<bevy::gilrs::GilrsPlugin>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_asset_base_path_with_env() {
        // When BEVY_ASSET_PATH is set, use it
        let result = get_asset_base_path(Some("/custom/path".to_string()), "/ignored");
        assert_eq!(result, "/custom/path");
    }

    #[test]
    fn test_get_asset_base_path_fallback() {
        // When BEVY_ASSET_PATH is not set, walk up ../.. to get to workspace root
        let result = get_asset_base_path(None, "/project/crates/mitchty");
        assert_eq!(result, "/project");
    }

    #[test]
    fn test_get_asset_base_path_fallback_relative() {
        // Two parents up from this crates path should be cwd/../.. == workspace root.
        let result = get_asset_base_path(None, "crates/mitchty");
        assert_eq!(result, "");
    }

    #[test]
    fn test_asset_path_debug() {
        // Workspace root is whats used for debug builds to get to the
        // AssetPlugin base path and so that matches what is in git.
        assert_eq!(
            asset_path_debug("test.png"),
            "crates/mitchty/src/assets/test.png"
        );
        assert_eq!(
            asset_path_debug("environment_maps/pisa.ktx2"),
            "crates/mitchty/src/assets/environment_maps/pisa.ktx2"
        );
        assert_eq!(
            asset_path_debug("foo/bar/baz/asset.ktx2"),
            "crates/mitchty/src/assets/foo/bar/baz/asset.ktx2"
        );
    }

    #[test]
    fn test_asset_path_release() {
        assert_eq!(
            asset_path_release("test.png"),
            "embedded://mitchty/assets/test.png"
        );
        assert_eq!(
            asset_path_release("environment_maps/pisa.ktx2"),
            "embedded://mitchty/assets/environment_maps/pisa.ktx2"
        );
        assert_eq!(
            asset_path_release("foo/bar/baz/asset.ktx2"),
            "embedded://mitchty/assets/foo/bar/baz/asset.ktx2"
        );
    }

    #[test]
    fn test_asset_path_wrapper() {
        let path = "environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2";
        let result = asset_path(path);

        // In debug builds, should use debug path
        #[cfg(debug_assertions)]
        assert_eq!(result, asset_path_debug(path));

        // In release builds, should use release path
        #[cfg(not(debug_assertions))]
        assert_eq!(result, asset_path_release(path));
    }
}
