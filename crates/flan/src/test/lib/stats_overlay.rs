#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use std::sync::{Arc, Mutex};

    use bevy::asset::RenderAssetUsages;
    use bevy::camera::{ClearColorConfig, RenderTarget};
    use bevy::prelude::*;
    use bevy::render::render_resource::BufferUsages;
    use bevy::render::storage::ShaderStorageBuffer;

    use crate::shaders::ShadersPlugin;
    use crate::stats::overlay::{
        StatsOverlayMaterial, StatsOverlayMaterialPlugin, StatsOverlayParams,
        StatsOverlayTextureMaterial, build_fps_points_image,
    };
    use crate::test::lib::GPU_RENDER_LOCK;
    use crate::test::lib::bevy::{
        CaptureShared, CaptureState, HeadlessCapturePlugin, RENDER_SIZE, RenderedFrame,
        make_render_target, run_and_capture,
    };
    use crate::{SlugAtlasLayout, SlugRunDesc, build_glyph_layout_image, build_runs_image};

    fn build_headless_overlay_app() -> App {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(
                        bevy::render::settings::WgpuSettings {
                            backends: Some(
                                bevy::render::settings::Backends::VULKAN
                                    | bevy::render::settings::Backends::METAL
                                    | bevy::render::settings::Backends::DX12
                                    | bevy::render::settings::Backends::GL,
                            ),
                            ..default()
                        },
                    ),
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    primary_cursor_options: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    close_when_requested: false,
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        app.add_plugins(ShadersPlugin);
        app.add_plugins(StatsOverlayMaterialPlugin);
        app
    }

    /// Build atlas, shape `fps_text`, and return layout + run + run_desc.
    /// Used by both render functions to avoid duplication.
    fn build_test_data(
        font_bytes: &[u8],
        fps_text: &str,
    ) -> (SlugAtlasLayout, crate::slug::SlugTextRun, SlugRunDesc) {
        let fps_chars = "0123456789. fps";
        let mut atlas = crate::slug::SlugAtlas::default();
        let font_id = atlas
            .register_font(font_bytes.to_vec())
            .expect("font registration must succeed");
        atlas.validate_glyphs(font_id, fps_chars);
        let ids = atlas.collect_glyph_ids(font_id, fps_chars);
        atlas.build_frame_atlas(&[(font_id, ids)]);
        let run = atlas
            .shape(font_id, fps_text, RENDER_SIZE as f32, [0, 0, 0, 255])
            .expect("shape must succeed");
        let layout = SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };
        let run_desc = SlugRunDesc {
            natural_advance: run.natural_advance,
            natural_height: run.natural_height,
            glyph_offset: 0,
            glyph_count: run.glyph_layout.len() as u32,
        };
        (layout, run, run_desc)
    }

    /// Derive a display-stable [min, max] fps range from test data.
    fn fps_range(fps_values: &[f32; 256]) -> (f32, f32) {
        let lo = fps_values
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            .max(0.0);
        let hi = fps_values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
            .max(0.0);
        if (hi - lo).abs() < 1.0 {
            (lo - 1.0, hi + 1.0)
        } else {
            (lo, hi)
        }
    }

    fn test_params(fps_values: &[f32; 256]) -> StatsOverlayParams {
        let (min_fps, max_fps) = fps_range(fps_values);
        StatsOverlayParams {
            node_size: Vec2::splat(RENDER_SIZE as f32),
            min_fps,
            max_fps,
            line_width: 0.01,
            layout_flags: 0x0C, // SLUG_LAYOUT_HFILL - stretch text to fill the text region
            alpha_discard: 0.01,
            _pad: 0.0,
            text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            background_color: Vec4::new(1.0, 1.0, 1.0, 1.0),
        }
    }

    fn spawn_ui_node(commands: &mut Commands, mat: impl Bundle, ih: Handle<Image>) {
        let cam = commands
            .spawn((
                Camera2d,
                Camera {
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::from(ih),
            ))
            .id();

        commands
            .spawn((
                Node {
                    width: Val::Px(RENDER_SIZE as f32),
                    height: Val::Px(RENDER_SIZE as f32),
                    ..default()
                },
                bevy::ui::UiTargetCamera(cam),
            ))
            .with_children(|p| {
                p.spawn((
                    mat,
                    Node {
                        width: Val::Px(RENDER_SIZE as f32),
                        height: Val::Px(RENDER_SIZE as f32),
                        ..default()
                    },
                ));
            });
    }

    pub fn render_stats_overlay(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_overlay_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (layout, run, run_desc) = build_test_data(font_bytes, fps_text);
        let params = test_params(fps_values);

        let mk = |data: &[u8], bufs: &mut Assets<ShaderStorageBuffer>| {
            let mut b = ShaderStorageBuffer::new(data, RenderAssetUsages::RENDER_WORLD);
            b.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
            bufs.add(b)
        };

        let (curves_h, ci_h, glyphs_h, runs_h, layout_h, fps_h) = {
            let mut bufs = app
                .world_mut()
                .resource_mut::<Assets<ShaderStorageBuffer>>();
            let layout_bytes: Vec<u8> = bytemuck::cast_slice(&run.glyph_layout).to_vec();
            let fps_bytes: Vec<u8> = fps_values.iter().flat_map(|f| f.to_le_bytes()).collect();
            (
                mk(&layout.curves_data, &mut bufs),
                mk(&layout.curve_indices_data, &mut bufs),
                mk(&layout.glyphs_data, &mut bufs),
                mk(bytemuck::bytes_of(&run_desc), &mut bufs),
                mk(&layout_bytes, &mut bufs),
                mk(&fps_bytes, &mut bufs),
            )
        };

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut mats: ResMut<Assets<StatsOverlayMaterial>>| {
                let mat = MaterialNode(mats.add(StatsOverlayMaterial {
                    params: params.clone(),
                    fps_points: fps_h.clone(),
                    curves: curves_h.clone(),
                    curve_indices: ci_h.clone(),
                    glyphs: glyphs_h.clone(),
                    runs: runs_h.clone(),
                    glyph_layout: layout_h.clone(),
                }));
                spawn_ui_node(&mut commands, mat, ih.clone());
            },
        );

        run_and_capture(&mut app, &state)
    }

    pub fn render_stats_overlay_texture(
        font_bytes: &[u8],
        fps_values: &[f32; 256],
        fps_text: &str,
    ) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_overlay_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (layout, run, run_desc) = build_test_data(font_bytes, fps_text);
        let params = test_params(fps_values);

        let (curves_h, ci_h, glyphs_h, runs_h, layout_h, fps_h) = {
            let mut imgs = app.world_mut().resource_mut::<Assets<Image>>();
            (
                imgs.add(layout.curves_image()),
                imgs.add(layout.curve_indices_image()),
                imgs.add(layout.glyphs_image()),
                imgs.add(build_runs_image(&run_desc)),
                imgs.add(build_glyph_layout_image(&run.glyph_layout)),
                imgs.add(build_fps_points_image(fps_values)),
            )
        };

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut mats: ResMut<Assets<StatsOverlayTextureMaterial>>| {
                let mat = MaterialNode(mats.add(StatsOverlayTextureMaterial {
                    params: params.clone(),
                    fps_points_image: fps_h.clone(),
                    curves_image: curves_h.clone(),
                    curve_indices_image: ci_h.clone(),
                    glyphs_image: glyphs_h.clone(),
                    runs_image: runs_h.clone(),
                    glyph_layout_image: layout_h.clone(),
                }));
                spawn_ui_node(&mut commands, mat, ih.clone());
            },
        );

        run_and_capture(&mut app, &state)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use inner::{render_stats_overlay, render_stats_overlay_texture};
