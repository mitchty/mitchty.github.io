// Shaders plugin for flan for shader assets and management of em.
use bevy::app::{App, Plugin};
use bevy::shader::Shader;

use super::{
    cartoon_filter_shader_handle, chromatic_aberration_shader_handle, edge_cartoon_shader_handle,
    em_interference_shader_handle, oil_painting_shader_handle, plot_default_shader_handle,
    plot_texture_shader_handle, slug_text_default_shader_handle, slug_text_texture_shader_handle,
    slug_text3d_default_shader_handle, slug_text3d_texture_shader_handle,
    stats_overlay_default_shader_handle, stats_overlay_texture_shader_handle,
    vhs_effect_shader_handle,
};

pub struct ShadersPlugin;

impl Plugin for ShadersPlugin {
    fn build(&self, app: &mut App) {
        // Fullscreen post-process shaders first, no reason, also need to
        // probably move this stuff into build.rs too after adding ui/2d/3d
        // feature flags to flan to control what materials are built.
        {
            let mut shaders = app
                .world_mut()
                .resource_mut::<bevy::asset::Assets<Shader>>();
            let _ = shaders.insert(
                chromatic_aberration_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::chromatic_aberration::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/chromatic_aberration.wgsl",
                ),
            );
            let _ = shaders.insert(
                vhs_effect_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::vhs_effect::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/vhs_effect.wgsl",
                ),
            );
            let _ = shaders.insert(
                em_interference_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::em_interference::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/em_interference.wgsl",
                ),
            );
            let _ = shaders.insert(
                oil_painting_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::oil_painting::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/oil_painting.wgsl",
                ),
            );
            let _ = shaders.insert(
                edge_cartoon_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::edge_cartoon::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/edge_cartoon.wgsl",
                ),
            );
            let _ = shaders.insert(
                cartoon_filter_shader_handle().id(),
                Shader::from_wgsl(
                    super::fullscreen::cartoon_filter::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/fullscreen/cartoon_filter.wgsl",
                ),
            );
        }

        // Plot shaders.
        {
            let mut shaders = app
                .world_mut()
                .resource_mut::<bevy::asset::Assets<Shader>>();
            let _ = shaders.insert(
                plot_default_shader_handle().id(),
                Shader::from_wgsl(
                    super::plot::plot_default::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/plot_default_wgsl_rs.wgsl",
                ),
            );
            let _ = shaders.insert(
                plot_texture_shader_handle().id(),
                Shader::from_wgsl(
                    super::plot::plot_texture::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/plot_texture_wgsl_rs.wgsl",
                ),
            );
        }

        // Slug text wgsl-rs shaders.
        {
            let mut shaders = app
                .world_mut()
                .resource_mut::<bevy::asset::Assets<Shader>>();

            // flan::slug::text - explicit default/texture modules (no macro_rules)
            let _ = shaders.insert(
                slug_text_default_shader_handle().id(),
                Shader::from_wgsl(
                    super::slug::text::slug_text_default::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/slug_text_default_wgsl_rs.wgsl",
                ),
            );
            let _ = shaders.insert(
                slug_text_texture_shader_handle().id(),
                Shader::from_wgsl(
                    super::slug::text::slug_text_texture::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/slug_text_texture_wgsl_rs.wgsl",
                ),
            );

            // flan::slug::text3dexplicit default/texture module bs
            let _ = shaders.insert(
                slug_text3d_default_shader_handle().id(),
                Shader::from_wgsl(
                    super::slug::text3d::slug_text3d_default::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/slug_text3d_default_wgsl_rs.wgsl",
                ),
            );
            let _ = shaders.insert(
                slug_text3d_texture_shader_handle().id(),
                Shader::from_wgsl(
                    super::slug::text3d::slug_text3d_texture::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/slug_text3d_texture_wgsl_rs.wgsl",
                ),
            );
        }

        // Stats overlay shader, just a combined plot and slug shader to act as first "api" consumer of all this junk
        {
            let mut shaders = app
                .world_mut()
                .resource_mut::<bevy::asset::Assets<Shader>>();

            let _ = shaders.insert(
                stats_overlay_default_shader_handle().id(),
                Shader::from_wgsl(
                    super::stats_overlay::stats_overlay_default::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/stats_overlay_default_wgsl_rs.wgsl",
                ),
            );

            let _ = shaders.insert(
                stats_overlay_texture_shader_handle().id(),
                Shader::from_wgsl(
                    super::stats_overlay::stats_overlay_texture::WGSL_MODULE.wgsl_source(),
                    "flan/shaders/stats_overlay_texture_wgsl_rs.wgsl",
                ),
            );
        }
    }
}
