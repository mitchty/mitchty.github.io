// flan::test::lib::shader test helpers.
//
// Very thin dispatch layer: each submodule knows the fixture path and delegates
// to the per-shader test file for the actual render calls.
//
// Structure mirrors the shader hierarchy:
//   shader::canary        - solid-black fill render canary (infrastructure health check)
//   shader::plot          - wgsl-rs plot shader
//   shader::slug          - wgsl-rs slug text shader
//   shader::stats_overlay - stats overlay sparkline + fps text combined, how I want future to really be for future wgsl shaders.

#[cfg(not(target_arch = "wasm32"))]
pub mod canary {
    use crate::test::lib::bevy::RenderedFrame;

    pub fn render() -> Option<RenderedFrame> {
        crate::test::lib::canary::render_canary()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod plot {
    use crate::test::lib::bevy::RenderedFrame;

    pub const CANONICAL: &[&str] = &["plot", "canonical"];

    pub fn render_canonical() -> Option<RenderedFrame> {
        crate::test::lib::bevy::render_plot_ui_material_default()
    }

    pub fn render_texture() -> Option<RenderedFrame> {
        crate::test::lib::bevy::render_plot_ui_material_texture()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod slug {
    use crate::test::lib::bevy::render_slug_fps_like;
    use crate::test::lib::bevy::{
        RenderedFrame, render_slug_bevy_material_texture, render_slug_bevy_ui_material_texture,
    };
    #[cfg(not(feature = "webgl"))]
    use crate::test::lib::bevy::{render_slug_bevy_material, render_slug_bevy_ui_material};

    /// Canonical fixture: `tests/fixtures/slug/canonical.png`.
    pub const CANONICAL: &[&str] = &["slug", "canonical"];

    #[cfg(not(feature = "webgl"))]
    pub fn render_canonical(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_bevy_ui_material(font_bytes, text)
    }

    #[cfg(not(feature = "webgl"))]
    pub fn render_bevy_material(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_bevy_material(font_bytes, text)
    }

    #[cfg(not(feature = "webgl"))]
    pub fn render_bevy_ui_material(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_bevy_ui_material(font_bytes, text)
    }

    pub fn render_fps_like(font_bytes: &[u8]) -> Option<RenderedFrame> {
        render_slug_fps_like(font_bytes)
    }

    pub fn render_bevy_ui_material_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_bevy_ui_material_texture(font_bytes, text)
    }

    pub fn render_bevy_material_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_bevy_material_texture(font_bytes, text)
    }
}

/// Helpers for `flan::slug::text` shader snapshot tests.
///
/// `slug_text_default` is the canonical source-of-truth render via storage buffers on a native hardware gpu.
/// `slug_text_texture` is compared against that via SSIM for webgl bs validation
#[cfg(not(target_arch = "wasm32"))]
pub mod slug_text {
    use crate::test::lib::bevy::RenderedFrame;
    #[cfg(not(feature = "webgl"))]
    use crate::test::lib::slug_text::{render_slug_text_default, render_slug_text3d_default};
    use crate::test::lib::slug_text::{render_slug_text_texture, render_slug_text3d_texture};

    pub const CANONICAL: &[&str] = &["slug_text", "canonical"];

    #[cfg(not(feature = "webgl"))]
    pub fn render_canonical(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_text_default(font_bytes, text)
    }

    pub fn render_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_text_texture(font_bytes, text)
    }

    #[cfg(not(feature = "webgl"))]
    pub fn render_3d_canonical(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_text3d_default(font_bytes, text)
    }

    pub fn render_3d_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        render_slug_text3d_texture(font_bytes, text)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod stats_overlay {
    use crate::stats::overlay::StatsOverlayColorMode;
    use crate::test::lib::bevy::RenderedFrame;
    use crate::test::lib::stats_overlay::{render_stats_overlay, render_stats_overlay_texture};

    pub const CANONICAL: &[&str] = &["stats_overlay", "canonical"];
    pub const INVERT_CANONICAL: &[&str] = &["stats_overlay", "invert_canonical"];

    pub fn render_canonical(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        render_stats_overlay(
            font_bytes,
            fps_values,
            fps_text,
            StatsOverlayColorMode::Color,
        )
    }

    pub fn render_texture(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        render_stats_overlay_texture(
            font_bytes,
            fps_values,
            fps_text,
            StatsOverlayColorMode::Color,
        )
    }

    pub fn render_invert_canonical(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        render_stats_overlay(
            font_bytes,
            fps_values,
            fps_text,
            StatsOverlayColorMode::Invert,
        )
    }

    pub fn render_invert_texture(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        render_stats_overlay_texture(
            font_bytes,
            fps_values,
            fps_text,
            StatsOverlayColorMode::Invert,
        )
    }
}
