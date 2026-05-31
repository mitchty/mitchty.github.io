// Shaders plugin for flan - provides shader assets and management
use bevy::app::{App, Plugin};
use bevy::shader::Shader;

pub struct ShadersPlugin;

impl Plugin for ShadersPlugin {
    fn build(&self, app: &mut App) {
        // Library wesl shaders are added into the assetserver so that
        // `ModulePath` works with `ShaderCashe::set_shader` output is
        // registered in a way the wesl imports expect in bevy.
        //
        // e.g. package::flan::shaders::lib::input::fullscreen_effect ->
        // Absolute["flan","shaders","lib","input","fullscreen_effect"]
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
                    "flan/shaders/lib/types/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/bindings/fullscreen_effect.wesl"),
                    "flan/shaders/lib/bindings/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/input/fullscreen_effect.wesl"),
                    "flan/shaders/lib/input/fullscreen_effect.wesl",
                ),
                (
                    include_str!("lib/types/plot.wesl"),
                    "flan/shaders/lib/types/plot.wesl",
                ),
                (
                    include_str!("lib/bindings/plot.wesl"),
                    "flan/shaders/lib/bindings/plot.wesl",
                ),
                (
                    include_str!("lib/input/plot.wesl"),
                    "flan/shaders/lib/input/plot.wesl",
                ),
                (
                    include_str!("lib/helpers/plot.wesl"),
                    "flan/shaders/lib/helpers/plot.wesl",
                ),
                // Slug font renderer library modules.
                (
                    include_str!("lib/slug/types.wesl"),
                    "flan/shaders/lib/slug/types.wesl",
                ),
                (
                    include_str!("lib/slug/math.wesl"),
                    "flan/shaders/lib/slug/math.wesl",
                ),
                (
                    include_str!("lib/slug/render.wesl"),
                    "flan/shaders/lib/slug/render.wesl",
                ),
                (
                    include_str!("lib/slug/text.wesl"),
                    "flan/shaders/lib/slug/text.wesl",
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
        bevy::asset::embedded_asset!(app, "slug/text.wesl");
        bevy::asset::embedded_asset!(app, "slug/ui_text.wesl");
        bevy::asset::embedded_asset!(app, "slug/mesh3d.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/chromatic-aberration.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/vhs-effect.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/em-interference.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/oil-painting.wesl");
        bevy::asset::embedded_asset!(app, "fullscreen/edge-cartoon.wesl");
        bevy::asset::embedded_asset!(app, "mesh/effect.wesl");
    }
}
