// flan::test::lib::slug_text headless render functions for the
// flan::slug::text and flan::slug::text3d shaders.

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use std::sync::{Arc, Mutex};

    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::render_resource::BufferUsages;
    use bevy::render::storage::ShaderBuffer;

    use crate::shaders::ShadersPlugin;
    use crate::slug_text_material::SlugTextMaterialPlugin;
    use crate::test::lib::GPU_RENDER_LOCK;
    use crate::test::lib::bevy::{
        CaptureShared, CaptureState, HeadlessCapturePlugin, RENDER_SIZE, RenderedFrame,
        build_atlas_and_run, make_render_target, run_and_capture,
    };
    use crate::{SlugAtlasLayout, SlugRunDesc, build_glyph_layout_image, build_runs_image};

    fn build_headless_slug_text_app() -> App {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                        bevy::render::settings::WgpuSettings {
                            backends: Some(
                                bevy::render::settings::Backends::VULKAN
                                    | bevy::render::settings::Backends::METAL
                                    | bevy::render::settings::Backends::DX12
                                    | bevy::render::settings::Backends::GL,
                            ),
                            ..default()
                        },
                    )),
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
        app.add_plugins(SlugTextMaterialPlugin);
        app
    }

    #[allow(clippy::type_complexity)]
    fn make_slug_text_images(
        images: &mut Assets<Image>,
        font_bytes: &[u8],
        text: &str,
    ) -> (
        Handle<Image>,
        Handle<Image>,
        Handle<Image>,
        Handle<Image>,
        Handle<Image>,
    ) {
        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);
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
        (
            images.add(layout.curves_image()),
            images.add(layout.curve_indices_image()),
            images.add(layout.glyphs_image()),
            images.add(build_runs_image(&run_desc)),
            images.add(build_glyph_layout_image(&run.glyph_layout)),
        )
    }

    #[allow(clippy::type_complexity)]
    #[cfg(not(feature = "webgl"))]
    fn make_slug_text_buffers(
        bufs: &mut Assets<ShaderBuffer>,
        font_bytes: &[u8],
        text: &str,
    ) -> (
        Handle<ShaderBuffer>,
        Handle<ShaderBuffer>,
        Handle<ShaderBuffer>,
        Handle<ShaderBuffer>,
        Handle<ShaderBuffer>,
        SlugAtlasLayout,
    ) {
        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);
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

        let mut mk = |data: &[u8]| {
            let mut b = ShaderBuffer::new(data, RenderAssetUsages::RENDER_WORLD);
            b.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
            bufs.add(b)
        };

        let layout_bytes: Vec<u8> = bytemuck::cast_slice(&run.glyph_layout).to_vec();
        let run_bytes: Vec<u8> = bytemuck::bytes_of(&run_desc).to_vec();

        let curves_h = mk(&layout.curves_data);
        let ci_h = mk(&layout.curve_indices_data);
        let glyphs_h = mk(&layout.glyphs_data);
        let runs_h = mk(&run_bytes);
        let layout_h = mk(&layout_bytes);

        (curves_h, ci_h, glyphs_h, runs_h, layout_h, layout)
    }

    fn spawn_ui_node(commands: &mut Commands, mat: impl Bundle, ih: Handle<Image>) {
        let cam = commands
            .spawn((
                Camera2d,
                crate::test::lib::bevy::headless_camera(),
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

    #[cfg(not(feature = "webgl"))]
    pub fn render_slug_text_default(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_text_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (curves_h, ci_h, glyphs_h, runs_h, layout_h, _layout) = make_slug_text_buffers(
            &mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(),
            font_bytes,
            text,
        );

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut mats: ResMut<Assets<crate::SlugTextMaterial>>| {
                let mat = MaterialNode(mats.add(crate::SlugTextMaterial {
                    params: crate::SlugParams {
                        node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: bevy::math::Mat4::IDENTITY.to_cols_array_2d(),
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

    pub fn render_slug_text_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_text_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (curves_h, ci_h, glyphs_h, runs_h, layout_h) = make_slug_text_images(
            &mut app.world_mut().resource_mut::<Assets<Image>>(),
            font_bytes,
            text,
        );

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands,
                  mut mats: ResMut<Assets<crate::SlugTextTextureMaterial>>| {
                let mat = MaterialNode(mats.add(crate::SlugTextTextureMaterial {
                    params: crate::SlugParams {
                        node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: bevy::math::Mat4::IDENTITY.to_cols_array_2d(),
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

    #[cfg(not(feature = "webgl"))]
    pub fn render_slug_text3d_default(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        use crate::{build_mesh_from_run, normalize_run_3d};
        use bevy::camera::visibility::NoFrustumCulling;

        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_text_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);
        let inv_h = 1.0 / run.natural_height.max(0.001);
        let norm_run = normalize_run_3d(&run, inv_h);
        let glyph_mesh = build_mesh_from_run(&norm_run, None);
        let aspect = norm_run.natural_advance.max(0.001);
        let local_to_clip =
            bevy::math::Mat4::orthographic_rh(-aspect * 0.5, aspect * 0.5, -0.5, 0.5, -1.0, 1.0);

        let layout = SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };

        let slug_params = crate::SlugParams {
            node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
            layout_flags: 0,
            alpha_discard: 0.01,
        };
        let text_color = bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0);
        let lc_arr = local_to_clip.to_cols_array_2d();

        // Pre-allocate the 96-byte params uniform buffer.
        let mut params_bytes = [0u8; 96];
        params_bytes[..16].copy_from_slice(bytemuck::bytes_of(&slug_params));
        params_bytes[16..32].copy_from_slice(bytemuck::bytes_of(&text_color));
        params_bytes[32..96].copy_from_slice(bytemuck::cast_slice(&lc_arr));

        // Three separate SSBs - one per atlas section (no sub-ranges).
        let (curves_h, ci_h, glyphs_h, params_buf_h) = {
            let mut bufs = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
            let mut mk = |data: &[u8]| {
                let mut b = ShaderBuffer::new(data, RenderAssetUsages::RENDER_WORLD);
                b.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
                bufs.add(b)
            };
            let mut pb = ShaderBuffer::new(&params_bytes, RenderAssetUsages::RENDER_WORLD);
            pb.buffer_description.usage = bevy::render::render_resource::BufferUsages::UNIFORM
                | bevy::render::render_resource::BufferUsages::COPY_DST;
            (
                mk(&layout.curves_data),
                mk(&layout.curve_indices_data),
                mk(&layout.glyphs_data),
                bufs.add(pb),
            )
        };

        let mesh_handle = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(glyph_mesh);

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        let mh = mesh_handle;
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut mats: ResMut<Assets<crate::SlugText3dMaterial>>| {
                let mat = mats.add(crate::SlugText3dMaterial {
                    params: slug_params,
                    text_color,
                    local_to_clip: lc_arr,
                    params_buf: Some(params_buf_h.clone()),
                    curves: curves_h.clone(),
                    curve_indices: ci_h.clone(),
                    glyphs: glyphs_h.clone(),
                    ..default()
                });
                commands.spawn((
                    Mesh3d(mh.clone()),
                    MeshMaterial3d(mat),
                    Transform::default(),
                    NoFrustumCulling,
                ));
                commands.spawn((
                    Camera3d::default(),
                    crate::test::lib::bevy::headless_camera(),
                    RenderTarget::from(ih.clone()),
                ));
            },
        );

        run_and_capture(&mut app, &state)
    }

    pub fn render_slug_text3d_texture(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        use crate::{build_mesh_from_run, normalize_run_3d};
        use bevy::camera::visibility::NoFrustumCulling;

        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_text_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);
        let inv_h = 1.0 / run.natural_height.max(0.001);
        let norm_run = normalize_run_3d(&run, inv_h);
        let glyph_mesh = build_mesh_from_run(&norm_run, None);
        let aspect = norm_run.natural_advance.max(0.001);
        let local_to_clip =
            bevy::math::Mat4::orthographic_rh(-aspect * 0.5, aspect * 0.5, -0.5, 0.5, -1.0, 1.0);

        let layout = SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };

        let (curves_h, ci_h, glyphs_h) = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            (
                images.add(layout.curves_image()),
                images.add(layout.curve_indices_image()),
                images.add(layout.glyphs_image()),
            )
        };
        let mesh_handle = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(glyph_mesh);

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        let lc = local_to_clip;
        let mh = mesh_handle;
        app.add_systems(
            Startup,
            move |mut commands: Commands,
                  mut mats: ResMut<Assets<crate::SlugText3dTextureMaterial>>| {
                let mat = mats.add(crate::SlugText3dTextureMaterial {
                    params: crate::SlugParams {
                        node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: lc.to_cols_array_2d(),
                    curves_image: curves_h.clone(),
                    curve_indices_image: ci_h.clone(),
                    glyphs_image: glyphs_h.clone(),
                    ..default()
                });
                commands.spawn((
                    Mesh3d(mh.clone()),
                    MeshMaterial3d(mat),
                    Transform::default(),
                    NoFrustumCulling,
                ));
                commands.spawn((
                    Camera3d::default(),
                    crate::test::lib::bevy::headless_camera(),
                    RenderTarget::from(ih.clone()),
                ));
            },
        );

        run_and_capture(&mut app, &state)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use inner::{render_slug_text_texture, render_slug_text3d_texture};

#[cfg(all(not(target_arch = "wasm32"), not(feature = "webgl")))]
pub use inner::{render_slug_text_default, render_slug_text3d_default};
