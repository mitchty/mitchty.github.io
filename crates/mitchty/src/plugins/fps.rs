//! FPS overlay plugin stats overlay shader specialization.
#[cfg(not(feature = "webgl"))]
use bevy::asset::RenderAssetUsages;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
#[cfg(not(feature = "webgl"))]
use bevy::render::{
    Render, RenderApp, RenderSystems,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_asset::RenderAssets,
    render_resource::BufferUsages,
    renderer::RenderQueue,
    storage::{GpuShaderBuffer, ShaderBuffer},
};
#[cfg(not(feature = "webgl"))]
use bytemuck;

use flan::{
    SlugAtlasLayout, SlugRunDesc, StatsOverlayData, StatsOverlayHandle, StatsOverlayMaterial,
    StatsOverlayMaterialPlugin, StatsOverlayParams,
};

/// Marker component: while present the FPS stats overlay is visible.
#[derive(Component, Default)]
pub struct FpsDisplay;

#[derive(Component)]
pub(crate) struct StatsOverlayNode;

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct OverlayFpsHandle(Option<Handle<ShaderBuffer>>);

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct OverlayRunsHandle(Option<Handle<ShaderBuffer>>);

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct OverlayGlyphLayoutHandle(Option<Handle<ShaderBuffer>>);

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct UploadFps {
    bytes: Vec<u8>,
}

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct UploadRuns {
    bytes: Vec<u8>,
}

#[cfg(not(feature = "webgl"))]
#[derive(Resource, Clone, Default, ExtractResource)]
struct UploadGlyphLayout {
    bytes: Vec<u8>,
}

#[cfg(feature = "webgl")]
#[derive(Resource, Default)]
struct OverlayTextureImages {
    fps: Option<Handle<Image>>,
    runs: Option<Handle<Image>>,
    layout: Option<Handle<Image>>,
}

#[derive(Resource)]
struct OverlayAtlas {
    atlas: flan::slug::SlugAtlas,
    font_id: flan::slug::FontId,
    font_size: f32,
}

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        if bavy::disabled::is_disabled(app.world(), "plot") {
            return;
        }

        let font_bytes = include_bytes!("../assets/fonts/FiraMono-Medium.ttf").to_vec();

        app.add_plugins(StatsOverlayMaterialPlugin)
            .init_resource::<StatsOverlayData>()
            .add_systems(PostStartup, reposition_overlay_node)
            .add_systems(Update, sync_overlay_visibility)
            .add_systems(
                Update,
                sample_fps_for_overlay.run_if(
                    bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(753))
                        .and_then(bevy::ecs::schedule::common_conditions::any_with_component::<FpsDisplay>),
                ),
            )
            .add_systems(Update, toggle_fps_display);

        #[cfg(not(feature = "webgl"))]
        {
            app.init_resource::<OverlayFpsHandle>()
                .init_resource::<OverlayRunsHandle>()
                .init_resource::<OverlayGlyphLayoutHandle>()
                .init_resource::<UploadFps>()
                .init_resource::<UploadRuns>()
                .init_resource::<UploadGlyphLayout>()
                .add_plugins(ExtractResourcePlugin::<OverlayFpsHandle>::default())
                .add_plugins(ExtractResourcePlugin::<OverlayRunsHandle>::default())
                .add_plugins(ExtractResourcePlugin::<OverlayGlyphLayoutHandle>::default())
                .add_plugins(ExtractResourcePlugin::<UploadFps>::default())
                .add_plugins(ExtractResourcePlugin::<UploadRuns>::default())
                .add_plugins(ExtractResourcePlugin::<UploadGlyphLayout>::default())
                .add_systems(Startup, setup_overlay_ssb(font_bytes));

            if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
                render_app.add_systems(
                    Render,
                    (upload_fps_ssb, upload_runs_ssb, upload_glyph_layout_ssb)
                        .in_set(RenderSystems::Queue),
                );
            }
        }

        #[cfg(feature = "webgl")]
        app.init_resource::<OverlayTextureImages>()
            .add_systems(Startup, setup_overlay_texture(font_bytes));
    }
}

#[allow(clippy::type_complexity)]
#[cfg(not(feature = "webgl"))]
fn setup_overlay_ssb(
    font_bytes: Vec<u8>,
) -> impl Fn(
    Commands,
    ResMut<Assets<ShaderBuffer>>,
    ResMut<Assets<StatsOverlayMaterial>>,
    ResMut<OverlayFpsHandle>,
    ResMut<OverlayRunsHandle>,
    ResMut<OverlayGlyphLayoutHandle>,
) {
    move |mut commands, mut buffers, mut materials, mut fps_h, mut runs_h, mut layout_h| {
        let (atlas_res, initial_run, layout) = build_atlas(&font_bytes);

        let mk = |data: &[u8]| {
            let mut b = ShaderBuffer::new(data, RenderAssetUsages::RENDER_WORLD);
            b.buffer_description.usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
            b
        };

        let curves_buf = buffers.add(mk(&layout.curves_data));
        let curve_indices_buf = buffers.add(mk(&layout.curve_indices_data));
        let glyphs_buf = buffers.add(mk(&layout.glyphs_data));

        let run_desc = SlugRunDesc {
            natural_advance: initial_run.natural_advance,
            natural_height: initial_run.natural_height,
            glyph_offset: 0,
            glyph_count: initial_run.glyph_layout.len() as u32,
        };
        let runs_buf = buffers.add(mk(bytemuck::bytes_of(&run_desc)));
        runs_h.0 = Some(runs_buf.clone());

        const MAX_FPS_GLYPHS: usize = 12;
        let init_layout = bytemuck::cast_slice::<_, u8>(&initial_run.glyph_layout).to_vec();
        let cap = init_layout.len().max(MAX_FPS_GLYPHS * 48);
        let mut layout_alloc = vec![0u8; cap];
        layout_alloc[..init_layout.len()].copy_from_slice(&init_layout);
        let glyph_layout_buf = buffers.add(mk(&layout_alloc));
        layout_h.0 = Some(glyph_layout_buf.clone());

        let zeros = vec![0u8; flan::STATS_OVERLAY_POINT_COUNT * 4];
        let fps_buf = buffers.add(mk(&zeros));
        fps_h.0 = Some(fps_buf.clone());

        let mat = materials.add(StatsOverlayMaterial {
            params: StatsOverlayParams::default(),
            fps_points: fps_buf,
            curves: curves_buf,
            curve_indices: curve_indices_buf,
            glyphs: glyphs_buf,
            runs: runs_buf,
            glyph_layout: glyph_layout_buf,
        });

        let handle = StatsOverlayHandle::Default(mat.clone());
        commands.insert_resource(handle);
        commands.insert_resource(atlas_res);
        spawn_overlay_node(&mut commands, MaterialNode(mat));
    }
}

#[cfg(feature = "webgl")]
fn setup_overlay_texture(
    font_bytes: Vec<u8>,
) -> impl Fn(
    Commands,
    ResMut<Assets<Image>>,
    ResMut<Assets<flan::StatsOverlayTextureMaterial>>,
    ResMut<OverlayTextureImages>,
) {
    use flan::{
        StatsOverlayTextureMaterial, build_fps_points_image, build_glyph_layout_image,
        build_runs_image,
    };

    move |mut commands, mut images, mut materials, mut tex_images| {
        let (atlas_res, initial_run, layout) = build_atlas(&font_bytes);

        let zeros = [0.0f32; 256];
        let fps_img_h = images.add(build_fps_points_image(&zeros));
        let curves_img_h = images.add(layout.curves_image());
        let ci_img_h = images.add(layout.curve_indices_image());
        let glyphs_img_h = images.add(layout.glyphs_image());

        let run_desc = SlugRunDesc {
            natural_advance: initial_run.natural_advance,
            natural_height: initial_run.natural_height,
            glyph_offset: 0,
            glyph_count: initial_run.glyph_layout.len() as u32,
        };
        let runs_img_h = images.add(build_runs_image(&run_desc));
        let layout_img_h = images.add(build_glyph_layout_image(&initial_run.glyph_layout));

        tex_images.fps = Some(fps_img_h.clone());
        tex_images.runs = Some(runs_img_h.clone());
        tex_images.layout = Some(layout_img_h.clone());

        let mat = materials.add(StatsOverlayTextureMaterial {
            params: StatsOverlayParams::default(),
            fps_points_image: fps_img_h,
            curves_image: curves_img_h,
            curve_indices_image: ci_img_h,
            glyphs_image: glyphs_img_h,
            runs_image: runs_img_h,
            glyph_layout_image: layout_img_h,
        });

        let handle = StatsOverlayHandle::Texture(mat.clone());
        commands.insert_resource(handle);
        commands.insert_resource(atlas_res);
        spawn_overlay_node(&mut commands, MaterialNode(mat));
    }
}

/// Build the font atlas and an initial shaped run.
fn build_atlas(font_bytes: &[u8]) -> (OverlayAtlas, flan::slug::SlugTextRun, SlugAtlasLayout) {
    let fps_chars = "0123456789. fps";
    let mut atlas = flan::slug::SlugAtlas::default();
    let font_id = atlas
        .register_font(font_bytes.to_vec())
        .expect("FiraMono-Medium must be valid TTF");
    atlas.validate_glyphs(font_id, fps_chars);
    let glyph_ids = atlas.collect_glyph_ids(font_id, fps_chars);
    atlas.build_frame_atlas(&[(font_id, glyph_ids)]);
    let font_size = 24.0_f32;

    let initial_run = atlas
        .shape(font_id, "00.0 fps", font_size, [0, 0, 0, 255])
        .expect("initial shape must succeed");

    let layout = SlugAtlasLayout {
        curves_data: atlas.frame.curves.clone(),
        curve_indices_data: atlas.frame.curve_indices.clone(),
        glyphs_data: atlas.frame.glyphs.clone(),
    };

    let atlas_res = OverlayAtlas {
        atlas,
        font_id,
        font_size,
    };
    (atlas_res, initial_run, layout)
}

/// Spawn the 230x40 UI node and attach the given `MaterialNode`.
fn spawn_overlay_node<M: Component>(commands: &mut Commands, mat_node: M) {
    commands.spawn((
        StatsOverlayNode,
        mat_node,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(10.0),
            top: Val::Px(10.0),
            width: Val::Px(230.0),
            height: Val::Px(40.0),
            ..default()
        },
        Visibility::Hidden,
    ));
}

fn reposition_overlay_node(
    ui_state: Res<crate::ui::state::UiState>,
    mut nodes: Query<&mut Node, With<StatsOverlayNode>>,
) {
    let top = if ui_state.backend == crate::ui::state::UiBackend::Egui {
        Val::Px(40.0)
    } else {
        Val::Px(10.0)
    };
    for mut node in nodes.iter_mut() {
        node.top = top;
    }
}

fn sync_overlay_visibility(
    fps_display: Query<(), With<FpsDisplay>>,
    mut overlay: Query<&mut Visibility, With<StatsOverlayNode>>,
) {
    let target = if fps_display.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    for mut vis in overlay.iter_mut() {
        if *vis != target {
            *vis = target;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sample_fps_for_overlay(
    diagnostics: Res<DiagnosticsStore>,
    mut data: ResMut<StatsOverlayData>,
    atlas: Res<OverlayAtlas>,
    handle: Res<StatsOverlayHandle>,
    mut materials: ResMut<Assets<StatsOverlayMaterial>>,
    mut tex_mats: ResMut<Assets<flan::StatsOverlayTextureMaterial>>,
    #[cfg(not(feature = "webgl"))] mut upload_fps: ResMut<UploadFps>,
    #[cfg(not(feature = "webgl"))] mut upload_runs: ResMut<UploadRuns>,
    #[cfg(not(feature = "webgl"))] mut upload_layout: ResMut<UploadGlyphLayout>,
    #[cfg(feature = "webgl")] tex_images: Res<OverlayTextureImages>,
    #[cfg(feature = "webgl")] mut images: ResMut<Assets<Image>>,
) {
    let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.value())
    else {
        return;
    };
    let fps = fps as f32;
    data.push_fps(fps);

    let run = atlas.atlas.shape(
        atlas.font_id,
        &format!("{fps:5.1} fps"),
        atlas.font_size,
        [0, 0, 0, 255],
    );

    handle.set_params(
        StatsOverlayParams {
            node_size: Vec2::new(230.0, 40.0),
            min_fps: data.display_min_fps(),
            max_fps: data.display_max_fps(),
            line_width: 0.01,
            layout_flags: 0x08,
            alpha_discard: 0.01,
            _pad: 0.0,
            text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            background_color: Vec4::ZERO,
        },
        &mut materials,
        &mut tex_mats,
    );

    #[cfg(not(feature = "webgl"))]
    {
        upload_fps.bytes = data.fps_points_bytes();
        if let Some(run) = &run {
            let run_desc = SlugRunDesc {
                natural_advance: run.natural_advance,
                natural_height: run.natural_height,
                glyph_offset: 0,
                glyph_count: run.glyph_layout.len() as u32,
            };
            upload_runs.bytes = bytemuck::bytes_of(&run_desc).to_vec();
            upload_layout.bytes = bytemuck::cast_slice(&run.glyph_layout).to_vec();
        }
    }

    #[cfg(feature = "webgl")]
    {
        use flan::{build_fps_points_image, build_glyph_layout_image, build_runs_image};

        let fps_pts = data.averaged_points();
        if let Some(h) = &tex_images.fps {
            if let Some(mut img) = images.get_mut(h) {
                *img = build_fps_points_image(&fps_pts);
            }
        }
        if let Some(run) = &run {
            let run_desc = SlugRunDesc {
                natural_advance: run.natural_advance,
                natural_height: run.natural_height,
                glyph_offset: 0,
                glyph_count: run.glyph_layout.len() as u32,
            };
            if let Some(h) = &tex_images.runs {
                if let Some(mut img) = images.get_mut(h) {
                    *img = build_runs_image(&run_desc);
                }
            }
            if let Some(h) = &tex_images.layout {
                if let Some(mut img) = images.get_mut(h) {
                    *img = build_glyph_layout_image(&run.glyph_layout);
                }
            }
        }
    }
}

#[cfg(not(feature = "webgl"))]
fn upload_fps_ssb(
    upload: Res<UploadFps>,
    handle: Res<OverlayFpsHandle>,
    gpu_bufs: Res<RenderAssets<GpuShaderBuffer>>,
    render_queue: Res<RenderQueue>,
) {
    if let (Some(h), false) = (&handle.0, upload.bytes.is_empty())
        && let Some(buf) = gpu_bufs.get(h.id())
    {
        render_queue.write_buffer(&buf.buffer, 0, &upload.bytes);
    }
}

#[cfg(not(feature = "webgl"))]
fn upload_runs_ssb(
    upload: Res<UploadRuns>,
    handle: Res<OverlayRunsHandle>,
    gpu_bufs: Res<RenderAssets<GpuShaderBuffer>>,
    render_queue: Res<RenderQueue>,
) {
    if let (Some(h), false) = (&handle.0, upload.bytes.is_empty())
        && let Some(buf) = gpu_bufs.get(h.id())
    {
        render_queue.write_buffer(&buf.buffer, 0, &upload.bytes);
    }
}

#[cfg(not(feature = "webgl"))]
fn upload_glyph_layout_ssb(
    upload: Res<UploadGlyphLayout>,
    handle: Res<OverlayGlyphLayoutHandle>,
    gpu_bufs: Res<RenderAssets<GpuShaderBuffer>>,
    render_queue: Res<RenderQueue>,
) {
    if let (Some(h), false) = (&handle.0, upload.bytes.is_empty())
        && let Some(buf) = gpu_bufs.get(h.id())
    {
        render_queue.write_buffer(&buf.buffer, 0, &upload.bytes);
    }
}

pub fn toggle_fps_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyF) {
        if let Ok(entity) = fps_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(FpsDisplay);
        }
    }
}
