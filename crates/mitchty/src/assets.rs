use bevy::prelude::*;

#[cfg(not(debug_assertions))]
use bevy::asset::embedded_asset;

/// Determine the base asset path for the AssetPlugin in debug builds.
/// Returns the path based on BEVY_ASSET_PATH env var or a fallback path.
///
/// Here to make testing with and without embedding easier.
#[allow(dead_code)] // Used only in specific build configurations
pub fn get_asset_base_path(bevy_asset_path_env: Option<String>, manifest_dir: &str) -> String {
    use std::path::PathBuf;

    bevy_asset_path_env.unwrap_or_else(|| {
        PathBuf::from(manifest_dir)
            .join("src")
            .join("assets")
            .to_string_lossy()
            .to_string()
    })
}

// Dead code allowed here cause its unit tested in both release/dev profiles but
// only ever used in one. Not a huge deal to have an extra function that isn't
// used in both profiles in the binary..

/// Get asset path for debug builds from fs or http if wasm
#[allow(dead_code)]
pub fn asset_path_debug(path: &str) -> String {
    path.to_string()
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

/// Plugin that configures assets based on build type and platform
pub struct AssetConfigPlugin;

impl Plugin for AssetConfigPlugin {
    fn build(&self, _app: &mut App) {
        // Only embed assets in release builds
        #[cfg(not(debug_assertions))]
        {
            embedded_asset!(
                _app,
                "assets/environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2"
            );
            embedded_asset!(
                _app,
                "assets/environment_maps/pisa_specular_rgb9e5_zstd.ktx2"
            );
        }
    }
}

// TODO: I might need to find out if for native I can use embedded reloading or
// not, saw it in the feature list.
pub fn create_default_plugins() -> bevy::app::PluginGroupBuilder {
    // In debug builds (native only), configure AssetPlugin to load from filesystem
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    {
        use bevy::asset::AssetPlugin;

        let asset_base = get_asset_base_path(
            std::env::var("BEVY_ASSET_PATH").ok(),
            env!("CARGO_MANIFEST_DIR"),
        );

        DefaultPlugins.set(AssetPlugin {
            file_path: asset_base,
            ..default()
        })
    }

    // WASM-specific configuration, basically sets window equal to the container its in size wise
    #[cfg(target_arch = "wasm32")]
    {
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                fit_canvas_to_parent: true,
                prevent_default_event_handling: false,
                ..default()
            }),
            ..default()
        })
    }

    // Default configuration for release native builds to abuse embedding.
    #[cfg(all(not(debug_assertions), not(target_arch = "wasm32")))]
    {
        DefaultPlugins.build()
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
        // When BEVY_ASSET_PATH is not set, use CARGO_MANIFEST_DIR/src/assets
        let result = get_asset_base_path(None, "/project/crates/mitchty");
        assert_eq!(result, "/project/crates/mitchty/src/assets");
    }

    #[test]
    fn test_get_asset_base_path_fallback_relative() {
        // Test with relative path
        let result = get_asset_base_path(None, "crates/mitchty");
        assert_eq!(result, "crates/mitchty/src/assets");
    }

    #[test]
    fn test_asset_path_debug() {
        assert_eq!(asset_path_debug("test.png"), "test.png");
        assert_eq!(
            asset_path_debug("environment_maps/pisa.ktx2"),
            "environment_maps/pisa.ktx2"
        );
        assert_eq!(
            asset_path_debug("foo/bar/baz/asset.ktx2"),
            "foo/bar/baz/asset.ktx2"
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
