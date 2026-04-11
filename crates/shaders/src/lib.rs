// For now this is its own crate separate from flan, but when I get things well
// enough along it'll be smashed into flan. Actually now that I think about it
// it might be easier to yeet this in there sooner rather than later.
use bevy::app::{App, Plugin};
use bevy::shader::Shader;

#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "render")]
pub mod snapshot;
#[cfg(feature = "render")]
pub mod wesl;

pub struct ShadersPlugin;

impl Plugin for ShadersPlugin {
    fn build(&self, app: &mut App) {
        // Library wesl shaders are added into the assetserver so that
        // `ModulePath` works with `ShaderCashe::set_shader` output is
        // registered in a way the wesl imports expect in bevy.
        //
        // e.g. package::shaders::lib::input::fullscreen_effect ->
        // Absolute["shaders","lib","input","fullscreen_effect"]
        //
        // TODO: see if there is way to use `load_internal_asset!` which can
        // prefix shaders to the `ModulePath`
        //
        // Library shaders are loaded by the wesl compiler by import path alone
        // not uuid handles so uuidv4 handles is ok nothing should use the
        // handles uuid directly.
        {
            let mut shaders = app
                .world_mut()
                .resource_mut::<bevy::asset::Assets<Shader>>();
            for (source, path) in [
                (
                    include_str!("lib/types/fullscreen_effect.wesl"),
                    "shaders/lib/types/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/bindings/fullscreen_effect.wesl"),
                    "shaders/lib/bindings/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/input/fullscreen_effect.wesl"),
                    "shaders/lib/input/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/types/plot.wesl"),
                    "shaders/lib/types/plot.wesl",
                ),
                (
                    include_str!("lib/bindings/plot.wesl"),
                    "shaders/lib/bindings/plot.wesl",
                ),
                (
                    include_str!("lib/input/plot.wesl"),
                    "shaders/lib/input/plot.wesl",
                ),
                (
                    include_str!("lib/helpers/plot.wesl"),
                    "shaders/lib/helpers/plot.wesl",
                ),
            ] {
                let id = bevy::asset::AssetId::Uuid {
                    uuid: bevy::asset::uuid::Uuid::new_v4(),
                };
                let _ = shaders.insert(id, Shader::from_wesl(source, path));
            }
        }

        // Entry shaders are treated as embedded assets still, the handles are
        // for runtime lookup.
        bevy::asset::embedded_asset!(app, "2d/plot.wesl");
        bevy::asset::embedded_asset!(app, "2d/reference.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/chromatic-aberration.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/vhs-effect.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/em-interference.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/oil-painting.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/edge-cartoon.wesl");
        bevy::asset::embedded_asset!(app, "mesh/effect.wesl");
    }
}
