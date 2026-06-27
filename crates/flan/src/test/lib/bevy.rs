// flan::test::lib::bevy headless Bevy render harness for shader testing and image output.
//
// Spin up a minimal Bevy App for each render call, drive it until the capture
// system fires or CAPTURE_TIMEOUT results in a panic, then returns the pixels
// as a RenderedFrame for SSIM comparison against the canonical render image
// from whenever things first ran from.
//

// Here be evil quiet the angy compiler about stuff that might not have a code
// path due to reasons but no need to fail a build with -D warnings on.
//
// This feels icky but its test code so whatever.

// TODO: This needs to get migrated to how the new fullscreen shaders work. Have
// to be sure the unit tests still run right in github actions first so only
// keeping the fullscreen stuff ported to the new less jank approach that bevy
// itself uses.
#![allow(dead_code, unused_imports)]

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use std::sync::{Arc, Mutex};

    use bevy::asset::RenderAssetUsages;
    use bevy::camera::{ClearColorConfig, RenderTarget};
    use bevy::prelude::*;
    use bevy::render::{
        Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{
            BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
            TexelCopyBufferInfo, TexelCopyBufferLayout, TextureDimension, TextureFormat,
            TextureUsages,
        },
        renderer::{RenderDevice, RenderQueue},
        settings::{Backends, RenderCreation, WgpuSettings},
        storage::ShaderBuffer,
        texture::GpuImage,
    };
    use bevy::ui::UiTargetCamera;
    use bevy::winit::WinitPlugin;

    use crate::SlugPlugin;
    #[cfg(not(feature = "webgl"))]
    use crate::SlugTextMaterial;
    use crate::shaders::ShadersPlugin;
    use crate::test::lib::{TEST_POINT_COUNT, has_no_adapter, test_sin_wave_points};
    use crate::{
        MAX_PLOT_POINTS, PlotPlugin, PlotPointsUniform, PlotUiMaterial, PlotUiMaterialTexture,
        PlotUniform,
    };

    // All Bevy headless renders share the GPU_RENDER_LOCK so they cannot run
    // concurrently with each other that caused mad issues so old tricks are the
    // best tricks.
    use crate::test::lib::GPU_RENDER_LOCK;

    /// Square render output size. 256x256 keeps PNG fixtures small and fast to
    /// render. I *may* need to change this later.
    pub const RENDER_SIZE: u32 = 256;

    /// RGBA pixel output from a render call.
    pub struct RenderedFrame {
        pub width: u32,
        pub height: u32,
        /// `width * height * 4` bytes, row-major, top-to-bottom.
        pub pixels: Vec<u8>,
    }

    /// Build and return:
    ///   - A `SlugAtlas` with the font registered and frame atlas built.
    ///   - The `FontId` of the registered font.
    ///   - The shaped run for `text`.
    ///
    /// Panics on font registration failure or empty run.
    pub fn build_atlas_and_run(
        font_bytes: &[u8],
        text: &str,
    ) -> (
        crate::slug::SlugAtlas,
        crate::slug::FontId,
        crate::slug::SlugTextRun,
    ) {
        let mut atlas = crate::slug::SlugAtlas::default();
        let fid = atlas
            .register_font(font_bytes.to_vec())
            .expect("font registration must succeed");

        atlas.validate_glyphs(fid, text);
        let ids = atlas.collect_glyph_ids(fid, text);
        atlas.build_frame_atlas(&[(fid, ids)]);

        let run = atlas
            .shape(fid, text, RENDER_SIZE as f32, [255, 255, 255, 255])
            .expect("shape() must succeed after build_frame_atlas");

        (atlas, fid, run)
    }

    /// Hard upper bound waiting for the capture system to fire. Mostly here in
    /// the case where a ci/gpu hangs somehow. Mostly here for ci tbh.
    const CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    pub type CaptureShared = Arc<Mutex<CaptureState>>;

    pub enum CaptureState {
        Pending,
        Done(Vec<u8>),
        Error(String),
    }

    #[derive(Resource)]
    struct CaptureResource {
        handle: Handle<Image>,
        state: CaptureShared,
        fired: bool,
        frame_count: u32,
        /// Minimum number of rendered frames before the capture is allowed to
        /// fire. Defaults to 0 aka fire as soon as any alpha > 0. Set to a higher
        /// value when post-process or other multi-frame pipeline stages need an
        /// extra cycle to initialize their bind groups before the capture is
        /// taken. All this bs is why I need to ditch this approach.
        min_frames: u32,
    }

    pub struct HeadlessCapturePlugin {
        pub handle: Handle<Image>,
        pub state: CaptureShared,
        /// See `CaptureResource::min_frames`. Defaults to 0.
        pub min_frames: u32,
    }

    impl HeadlessCapturePlugin {
        pub fn new(handle: Handle<Image>, state: CaptureShared) -> Self {
            Self {
                handle,
                state,
                min_frames: 0,
            }
        }
    }

    impl Plugin for HeadlessCapturePlugin {
        fn build(&self, app: &mut App) {
            let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
                return;
            };
            render_app
                .insert_resource(CaptureResource {
                    handle: self.handle.clone(),
                    state: self.state.clone(),
                    fired: false,
                    frame_count: 0,
                    min_frames: self.min_frames,
                })
                .add_systems(Render, capture_render_target.in_set(RenderSystems::Cleanup));
        }
    }

    fn capture_render_target(
        mut res: ResMut<CaptureResource>,
        gpu_images: Res<RenderAssets<GpuImage>>,
        device: Res<RenderDevice>,
        queue: Res<RenderQueue>,
    ) {
        if res.fired {
            return;
        }
        let Some(gpu_image) = gpu_images.get(&res.handle) else {
            return;
        };

        let w = RENDER_SIZE;
        let h = RENDER_SIZE;
        let bpp = 4u32;
        let unpadded = w * bpp;
        // wgpu::COPY_BYTES_PER_ROW_ALIGNMENT = 256 Bevy doesn't re-export it directly so be evil.
        let align = 256u32;
        let padded = unpadded.div_ceil(align) * align;

        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("bevy-capture-staging"),
            size: (padded * h) as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut enc = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("bevy-capture-enc"),
        });
        enc.copy_texture_to_buffer(
            gpu_image.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(enc.finish()));

        // Poll RenderDevice directly avoids touching wgpu_device() and the
        // version mismatch that seems to cause.
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        loop {
            device.poll(PollType::Poll).ok();
            match rx.try_recv() {
                Ok(Ok(())) => break,
                Ok(Err(e)) => {
                    *res.state.lock().expect("state mutex poisoned") =
                        CaptureState::Error(format!("buffer map failed: {e}"));
                    res.fired = true;
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    *res.state.lock().expect("state mutex poisoned") =
                        CaptureState::Error("map_async sender dropped".into());
                    res.fired = true;
                    return;
                }
            }
        }

        let mapped = staging.slice(..).get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h * bpp) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        staging.unmap();

        // Only commit once there is real rendered content in the target buffer.
        //
        // The render target clears to TRANSPARENT (0,0,0,0) every frame. Any
        // pixel written by a shader will have alpha > 0. If all pixels are
        // still transparent, the pipeline and bind groups aren't ready yet so
        // return without firing so we retry next frame. This replaces any
        // fixed frame-count wait: it naturally adapts to however many frames
        // the gpu and driver need, on any machine.
        let max_alpha = pixels.chunks_exact(4).map(|p| p[3]).max().unwrap_or(0);
        if max_alpha == 0 || res.frame_count < res.min_frames {
            // Diagnostics every 30 render frames so we can see if something
            // eventually changes or not. I do not yet know whats wrong on certain
            // platforms to cause this or not.
            if res.frame_count.is_multiple_of(30) {
                let non_zero_rgb = pixels
                    .chunks_exact(4)
                    .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
                    .count();
                eprintln!(
                    "bevy-capture frame {}: max_alpha={} min_frames={} non-zero-rgb pixels={}",
                    res.frame_count, max_alpha, res.min_frames, non_zero_rgb
                );
            }
            res.frame_count += 1;
            return;
        }
        eprintln!(
            "[bevy-capture] frame {}: content detected (max_alpha={}), firing",
            res.frame_count, max_alpha
        );

        *res.state.lock().expect("state mutex poisoned") = CaptureState::Done(pixels);
        res.fired = true;
    }

    /// Build a bare headless Bevy `App` with the render stack enabled but no
    /// window, no Winit, and no log output.
    /// one of those extra layers.
    pub fn build_headless_app_base() -> App {
        let mut app = App::new();
        app.add_plugins(
            // DefaultPlugins brings the full rendering stack (CorePipeline,
            // Sprite, UI, UiRender, Asset, Image, ...) without us having to
            // enumerate every sub-plugin. We just disable the pieces that
            // require a display and configure RenderPlugin for headless.
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                        backends: Some(
                            Backends::VULKAN | Backends::METAL | Backends::DX12 | Backends::GL,
                        ),
                        ..default()
                    })),
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                // Headless: no primary window, don't exit when none exists.
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    primary_cursor_options: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    close_when_requested: false,
                })
                // No window -> no Winit.
                .disable::<WinitPlugin>()
                // Don't spam test output with Bevy log lines.
                .disable::<bevy::log::LogPlugin>(),
        );
        app
    }

    fn build_headless_app() -> App {
        let mut app = build_headless_app_base();
        app.add_plugins((ShadersPlugin, PlotPlugin));
        app
    }

    /// Headless app for slug tests.
    ///
    /// `SlugPlugin` registers all three material pipelines and adds all the
    /// slug Bevy systems (validate -> build_frame_atlas -> upload_atlas ->
    /// sync_text_meshes -> sync_node_size -> sync_slug_3d_transforms).
    ///
    /// By letting `SlugPlugin` manage SSB creation through its normal Update
    /// systems, storage buffers are always created after `app.finish()` has
    /// initialized the render world - avoiding the "binding size is zero"
    /// wgpu error that occurs when SSBs are created before the render world.
    fn build_headless_slug_app() -> App {
        let mut app = build_headless_app_base();
        app.add_plugins((ShadersPlugin, SlugPlugin));
        app
    }

    pub fn make_render_target(images: &mut Assets<Image>) -> Handle<Image> {
        let size = Extent3d {
            width: RENDER_SIZE,
            height: RENDER_SIZE,
            depth_or_array_layers: 1,
        };
        let mut image = Image::new_fill(
            size,
            TextureDimension::D2,
            &[0u8; 4],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        );
        image.texture_descriptor.usage = TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC
            | TextureUsages::COPY_DST
            | TextureUsages::TEXTURE_BINDING;
        images.add(image)
    }

    /// Build a `Camera` for headless rendering to an image render target.
    ///
    /// Sets BOTH clear colors to transparent so neither the main-pass clear
    /// nor the output-write clear fills the render target with the world
    /// `ClearColor` (Bevy's default dark grey-blue `[43, 44, 47, 255]`).
    ///
    /// `Camera::default()` only sets `clear_color` the main-pass clear color.
    /// `CameraOutputMode::Write.clear_color` is a second pass color
    /// that also defaults to `ClearColorConfig::Default` as a world color.
    ///
    /// Without setting BOTH, the output-write step fills the image with the
    /// world clear color on every frame and produces `max_alpha = 255` from the
    /// `ClearColor` resource before any shader has even run, and then
    /// overwriting any actual render output with the grey-blue background and
    /// the test fails in 0.19 now.
    pub fn headless_camera() -> Camera {
        use bevy::camera::CameraOutputMode;
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: CameraOutputMode::Write {
                blend_state: None,
                clear_color: ClearColorConfig::Custom(Color::NONE),
            },
            ..default()
        }
    }

    /// Build the points storage buffer from the canonical test signal.
    ///
    /// Uses `Vec<Vec2>` + `ShaderBuffer::from` the same pattern I poc'd
    /// in mitchty so that the test exercises the real public SSB API.
    ///
    fn make_points_buffer(storage_buffers: &mut Assets<ShaderBuffer>) -> Handle<ShaderBuffer> {
        let points: Vec<Vec2> = test_sin_wave_points()
            .into_iter()
            .map(|[x, y]| Vec2::new(x, y))
            .collect();
        storage_buffers.add(ShaderBuffer::from(points))
    }

    /// Build a `PlotUniform` with the canonical test defaults.
    fn test_params() -> PlotUniform {
        PlotUniform {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            zoom: Vec2::ONE,
            offset: Vec2::ZERO,
            count: TEST_POINT_COUNT.min(MAX_PLOT_POINTS) as u32,
            line_width: 0.003,
            _pad: 0.00,
        }
    }

    pub fn run_and_capture(app: &mut App, state: &CaptureShared) -> Option<RenderedFrame> {
        // app.run() calls finish() + cleanup() before entering its loop.
        // When driving the app manually with update() we must call finish()
        // ourselves first that is where RenderPlugin initializes the wgpu
        // device and inserts RenderDevice into the RenderApp sub-world.
        // Without it every render-world system that requires RenderDevice panics.
        app.finish();
        app.cleanup();

        let start = std::time::Instant::now();
        loop {
            app.update();
            match &*state.lock().expect("state mutex poisoned") {
                CaptureState::Pending => {
                    let elapsed = start.elapsed();
                    if elapsed >= CAPTURE_TIMEOUT {
                        panic!(
                            "bevy capture timed out after {elapsed:.2?} \
                             limit: {CAPTURE_TIMEOUT:.2?} \
                             render pipeline may not have compiled or \
                             capture system never ran only the shadow knows"
                        );
                    }
                }
                _ => break,
            }
        }

        match std::mem::replace(
            &mut *state.lock().expect("state mutex poisoned"),
            CaptureState::Pending,
        ) {
            CaptureState::Done(pixels) => Some(RenderedFrame {
                width: RENDER_SIZE,
                height: RENDER_SIZE,
                pixels,
            }),
            CaptureState::Error(e) if has_no_adapter(&e) => {
                eprintln!("bevy render skipped - no GPU adapter: {e}");
                None
            }
            CaptureState::Error(e) => panic!("bevy capture failed: {e}"),
            CaptureState::Pending => unreachable!("loop exits only on non-pending"),
        }
    }

    /// Render `PlotUiMaterial` storage-buffer path, `@group(1)`.
    pub fn render_plot_ui_material_default() -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let points_handle =
            make_points_buffer(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        let ph = points_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut materials: ResMut<Assets<PlotUiMaterial>>| {
                let mat = materials.add(PlotUiMaterial {
                    params: test_params(),
                    points: ph.clone(),
                });

                // Spawn the camera first so we have its Entity to pass to TargetCamera.
                // TargetCamera directs the UI hierarchy to render into the camera's
                // RenderTarget rather than the absent primary window.
                let cam = commands
                    .spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())))
                    .id();

                // Use fixed pixel sizes rather than Percent with no primary window
                // the UI layout has no reference size to resolve percentages against.
                commands
                    .spawn((
                        Node {
                            width: Val::Px(RENDER_SIZE as f32),
                            height: Val::Px(RENDER_SIZE as f32),
                            ..default()
                        },
                        UiTargetCamera(cam),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            MaterialNode(mat),
                            Node {
                                width: Val::Px(RENDER_SIZE as f32),
                                height: Val::Px(RENDER_SIZE as f32),
                                ..default()
                            },
                        ));
                    });
            },
        );

        run_and_capture(&mut app, &state)
    }

    /// Render `PlotUiMaterialTexture` (uniform-array path, `@group(1)`).
    pub fn render_plot_ui_material_texture() -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut materials: ResMut<Assets<PlotUiMaterialTexture>>| {
                let points = test_sin_wave_points();
                let mut data = [bevy::math::Vec4::ZERO; MAX_PLOT_POINTS];
                for (i, [x, y]) in points.iter().enumerate().take(MAX_PLOT_POINTS) {
                    data[i] = bevy::math::Vec4::new(*x, *y, 0.0, 0.0);
                }
                let mat = materials.add(PlotUiMaterialTexture {
                    params: test_params(),
                    points: PlotPointsUniform { data },
                });

                let cam = commands
                    .spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())))
                    .id();

                commands
                    .spawn((
                        Node {
                            width: Val::Px(RENDER_SIZE as f32),
                            height: Val::Px(RENDER_SIZE as f32),
                            ..default()
                        },
                        UiTargetCamera(cam),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            MaterialNode(mat),
                            Node {
                                width: Val::Px(RENDER_SIZE as f32),
                                height: Val::Px(RENDER_SIZE as f32),
                                ..default()
                            },
                        ));
                    });
            },
        );

        run_and_capture(&mut app, &state)
    }

    /// Render `SlugTextMaterial` (UiMaterial, storage-buffer path) via the full
    /// `SlugPlugin` system chain. Exercises atlas upload, draw-buffer packing,
    /// and node-size sync through `SlugPlugin`'s Update systems.
    #[cfg(not(feature = "webgl"))]
    pub fn render_slug_bevy_material(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let font_bytes_owned = font_bytes.to_vec();
        let text_owned = text.to_owned();
        let ih = image_handle.clone();

        app.add_systems(
            Startup,
            move |mut commands: Commands,
                  mut atlas: ResMut<crate::slug::SlugAtlas>,
                  mut mats: ResMut<Assets<SlugTextMaterial>>| {
                let fid = atlas
                    .register_font(font_bytes_owned.clone())
                    .expect("render_slug_bevy_material: font registration must succeed");

                let mat = mats.add(SlugTextMaterial {
                    params: crate::SlugParams {
                        node_size: Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
                    ..default()
                });
                let cam = commands
                    .spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())))
                    .id();
                commands.spawn((
                    crate::SlugTextNode {
                        text: text_owned.clone(),
                        font_size: RENDER_SIZE as f32,
                        color: [0, 0, 0, 255],
                        layout: crate::Layout::default(),
                        depth: None,
                    },
                    crate::SlugTextFont(fid),
                    MaterialNode(mat),
                    Node {
                        width: Val::Px(RENDER_SIZE as f32),
                        height: Val::Px(RENDER_SIZE as f32),
                        ..default()
                    },
                    UiTargetCamera(cam),
                ));
            },
        );

        run_and_capture(&mut app, &state)
    }

    /// Build a headless app wired for the texture-binding material path:
    /// - `SlugTextTextureMaterial` for UI (`UiMaterial`)
    /// - `SlugText3dTextureMaterial` for 3D (`Material`)
    ///
    /// No `SlugPlugin` - just the two material plugins needed.
    fn build_headless_slug_texture_app() -> App {
        let mut app = build_headless_app_base();
        // Register the shader assets so the material pipelines can find them.
        app.add_plugins(ShadersPlugin);
        // Register the UI texture-binding material.
        app.add_plugins(UiMaterialPlugin::<
            crate::slug_text_material::SlugTextTextureMaterial,
        >::default());
        // Register the 3D texture-binding material.
        app.add_plugins(MaterialPlugin::<
            crate::slug_text_material::SlugText3dTextureMaterial,
        >::default());
        app
    }

    /// Build the five `Handle<Image>` assets needed by `SlugMaterialTexture`
    /// from the atlas + run data. Returns in field order.
    #[allow(clippy::type_complexity)]
    fn make_slug_texture_images(
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
        use crate::{SlugAtlasLayout, build_glyph_layout_image, build_runs_image};

        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);
        let layout = SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };

        let run_desc = crate::SlugRunDesc {
            natural_advance: run.natural_advance,
            natural_height: run.natural_height,
            glyph_offset: 0,
            glyph_count: run.glyph_layout.len() as u32,
        };

        let curves_h = images.add(layout.curves_image());
        let curve_indices_h = images.add(layout.curve_indices_image());
        let glyphs_h = images.add(layout.glyphs_image());
        let runs_h = images.add(build_runs_image(&run_desc));
        let glyph_layout_h = images.add(build_glyph_layout_image(&run.glyph_layout));

        (curves_h, curve_indices_h, glyphs_h, runs_h, glyph_layout_h)
    }

    /// Render `SlugMaterialTexture` as `UiMaterial` (`@group(1)`) headlessly.
    ///
    /// Exercises the texture-binding `AsBindGroup` impl + `slug_ui_material_webgl`
    /// shader through the full Bevy `UiMaterial` pipeline - the same code path
    /// used by the FPS overlay on webgl builds.
    pub fn render_slug_bevy_ui_material_texture(
        font_bytes: &[u8],
        text: &str,
    ) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_texture_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        let (curves_h, curve_indices_h, glyphs_h, runs_h, glyph_layout_h) =
            make_slug_texture_images(
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
            move |mut commands: Commands, mut mats: ResMut<Assets<crate::slug_text_material::SlugTextTextureMaterial>>| {
                let mat = mats.add(crate::slug_text_material::SlugTextTextureMaterial {
                    params: crate::SlugParams {
                        node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0),
                    curves_image: curves_h.clone(),
                    curve_indices_image: curve_indices_h.clone(),
                    glyphs_image: glyphs_h.clone(),
                    runs_image: runs_h.clone(),
                    glyph_layout_image: glyph_layout_h.clone(),
                    ..default()
                });

                let cam = commands
                    .spawn((
                        Camera2d,
                        headless_camera(),
                        RenderTarget::from(ih.clone()),
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
                            MaterialNode(mat),
                            Node {
                                width: Val::Px(RENDER_SIZE as f32),
                                height: Val::Px(RENDER_SIZE as f32),
                                ..default()
                            },
                        ));
                    });
            },
        );

        run_and_capture(&mut app, &state)
    }

    /// Render using `SlugText3dTextureMaterial` as a flat world-space `MeshMaterial3d`
    /// (`@group(3)`) headlessly.
    ///
    /// Mirrors exactly what `sync_text_meshes` does for flat 3d world-space
    /// text on the webgl path:
    ///   1. Normalize the run to 1 world-unit tall, Y-flipped, origin-centered
    ///      (`normalize_run_3d`).
    ///   2. Build the per-glyph-quad mesh with custom slug vertex attributes
    ///      (`build_mesh_from_run`).
    ///   3. Supply a `local_to_clip` ortho matrix that maps +/-0.5 world units
    ///      to NDC, bypassing the camera transform entirely.
    ///   4. Add `NoFrustumCulling` so world-space bounding-box checks against
    ///      the camera frustum cannot hide the mesh.
    pub fn render_slug_bevy_material_texture(
        font_bytes: &[u8],
        text: &str,
    ) -> Option<RenderedFrame> {
        use crate::{build_mesh_from_run, normalize_run_3d};
        use bevy::camera::visibility::NoFrustumCulling;

        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_texture_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        // Build atlas and run outside the Startup closure so we can compute
        // the mesh and local_to_clip before handing anything to the ECS.
        let (atlas, _fid, run) = build_atlas_and_run(font_bytes, text);

        // Normalize exactly as sync_text_meshes does for flat 3d text:
        // 1 world unit tall, Y-flipped, origin-centered.
        let inv_h = 1.0 / run.natural_height.max(0.001);
        let norm_run = normalize_run_3d(&run, inv_h);

        // Build the slug per-glyph-quad mesh with the custom vertex attributes
        // the slug_material_3d_webgl vertex shader expects.
        let glyph_mesh = build_mesh_from_run(&norm_run, None);

        // local_to_clip: ortho mapping ±0.5 world units -> NDC [-1, 1].
        // The normalized run is centered at the origin and spans:
        //   X: [-advance/2, +advance/2]   Y: [-0.5, +0.5]
        // An orthographic projection with left=-0.5, right=+0.5, bottom=-0.5,
        // top=+0.5 maps this exactly to NDC. We widen x slightly by the aspect
        // ratio (advance/height after normalization) so the full text is visible.

        // height is already 1.0
        let aspect = norm_run.natural_advance.max(0.001);
        let local_to_clip =
            bevy::math::Mat4::orthographic_rh(-aspect * 0.5, aspect * 0.5, -0.5, 0.5, -1.0, 1.0);

        // Build atlas image handles.
        let layout = crate::SlugAtlasLayout {
            curves_data: atlas.frame.curves.clone(),
            curve_indices_data: atlas.frame.curve_indices.clone(),
            glyphs_data: atlas.frame.glyphs.clone(),
        };
        let world = app.world_mut();
        let (curves_h, curve_indices_h, glyphs_h) = {
            let mut images = world.resource_mut::<Assets<Image>>();
            (
                images.add(layout.curves_image()),
                images.add(layout.curve_indices_image()),
                images.add(layout.glyphs_image()),
            )
        };
        let mesh_handle = world.resource_mut::<Assets<Mesh>>().add(glyph_mesh);

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
                  mut mats: ResMut<
                Assets<crate::slug_text_material::SlugText3dTextureMaterial>,
            >| {
                let mat = mats.add(crate::slug_text_material::SlugText3dTextureMaterial {
                    params: crate::SlugParams {
                        // node_size unused by the 3d mesh shader pixels_per_em
                        // comes from fwidth in the fragment shader, but filled
                        // in anyway for uniform completeness. I need to noodle
                        // better param inputs.
                        node_size: bevy::math::Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: bevy::math::Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: lc.to_cols_array_2d(),
                    curves_image: curves_h.clone(),
                    curve_indices_image: curve_indices_h.clone(),
                    glyphs_image: glyphs_h.clone(),
                    ..default()
                });

                // Spawn the glyph mesh. NoFrustumCulling ensures world-space AABB
                // checks against the camera frustum never cull the quads the
                // vertex shader positions them via local_to_clip, not the camera.
                commands.spawn((
                    Mesh3d(mh.clone()),
                    MeshMaterial3d(mat),
                    Transform::default(),
                    NoFrustumCulling,
                ));

                // Camera3d is required for the PBR pipeline to activate, but it
                // does not contribute to vertex transformation directly
                // local_to_clip handles that. Place it at the origin looking
                // forward.
                commands.spawn((
                    Camera3d::default(),
                    headless_camera(),
                    RenderTarget::from(ih.clone()),
                ));
            },
        );

        run_and_capture(&mut app, &state)
    }

    /// Render a slug text UIMaterial node:
    pub fn render_slug_fps_like(font_bytes: &[u8]) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let font_bytes_owned = font_bytes.to_vec();
        let ih = image_handle.clone();

        // Use the platform-native material type so sync_node_size picks it up.
        #[cfg(not(feature = "webgl"))]
        app.add_systems(
            Startup,
            move |mut commands: Commands,
                  mut atlas: ResMut<crate::slug::SlugAtlas>,
                  mut materials: ResMut<Assets<crate::SlugTextMaterial>>| {
                let fid = atlas
                    .register_font(font_bytes_owned.clone())
                    .expect("font registration");
                atlas.validate_glyphs(fid, "0123456789.fps ");

                let material = materials.add(crate::SlugTextMaterial {
                    params: crate::SlugParams {
                        node_size: Vec2::ZERO, // set by sync_node_size
                        layout_flags: crate::Layout::new()
                            .with_vertical(crate::Vertical::Center)
                            .with_horizontal(crate::Horizontal::Right)
                            .to_u32(),
                        alpha_discard: 0.01,
                    },
                    text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
                    ..default()
                });

                // The render target is 256x256 but give the node the same
                // dimensions mitchty uses so sync_node_size sees (130, 24) to
                // better test what the app uses.
                let cam = commands
                    .spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())))
                    .id();

                commands.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(130.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                    // NOTE: mitchty abuses Visibility::Hidden so the FPS display is
                    // hidden until deliberately displayed. That suppresses rendering in
                    // the capture harness, so we use Inherited (default/visible)
                    // here.
                    Visibility::Inherited,
                    MaterialNode(material),
                    crate::SlugTextNode {
                        text: "00.0 fps".into(),
                        font_size: 18.0,
                        color: [0, 0, 0, 255],
                        layout: crate::Layout::new()
                            .with_vertical(crate::Vertical::Center)
                            .with_horizontal(crate::Horizontal::Right),
                        depth: None,
                    },
                    crate::SlugTextFont(fid),
                    bevy::ui::UiTargetCamera(cam),
                ));
            },
        );

        #[cfg(feature = "webgl")]
        {
            let font_bytes_webgl = font_bytes.to_vec();
            let ih2 = image_handle.clone();
            app.add_systems(
                Startup,
                move |mut commands: Commands,
                mut atlas: ResMut<crate::slug::SlugAtlas>,
                mut materials: ResMut<Assets<crate::SlugTextTextureMaterial>>| {
                    let fid = atlas
                        .register_font(font_bytes_webgl.clone())
                        .expect("font registration texture path");
                    atlas.validate_glyphs(fid, "0123456789.fps ");
                    let material = materials.add(crate::SlugTextTextureMaterial::default());
                    let cam = commands
                        .spawn((
                            Camera2d,
                            headless_camera(),
                            RenderTarget::from(ih2.clone()),
                        ))
                        .id();
                    commands.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(130.0),
                            height: Val::Px(24.0),
                            ..default()
                        },
                        Visibility::Inherited,
                        MaterialNode(material),
                        crate::SlugTextNode {
                            text: "00.0 fps".into(),
                            font_size: 18.0,
                            color: [0, 0, 0, 255],
                            layout: crate::Layout::new()
                                .with_vertical(crate::Vertical::Center)
                                .with_horizontal(crate::Horizontal::Right),
                            depth: None,
                        },
                        crate::SlugTextFont(fid),
                        bevy::ui::UiTargetCamera(cam),
                    ));
                },
            );
        }

        run_and_capture(&mut app, &state)
    }

    /// Render `SlugTextMaterial` as `UiMaterial` (`@group(1)`) headlessly via the
    /// full `SlugPlugin` system chain. Exercises atlas upload, draw-buffer packing,
    /// and node-size sync through `SlugPlugin`'s Update systems.
    #[cfg(not(feature = "webgl"))]
    pub fn render_slug_bevy_ui_material(font_bytes: &[u8], text: &str) -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_slug_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let font_bytes_owned = font_bytes.to_vec();
        let text_owned = text.to_owned();
        let ih = image_handle.clone();

        app.add_systems(
            Startup,
            move |mut commands: Commands,
                  mut atlas: ResMut<crate::slug::SlugAtlas>,
                  mut mats: ResMut<Assets<SlugTextMaterial>>| {
                let fid = atlas
                    .register_font(font_bytes_owned.clone())
                    .expect("render_slug_bevy_ui_material: font registration must succeed");

                // Pre-attach MaterialNode so init_slug_entity skips this entity.
                let mat = mats.add(SlugTextMaterial {
                    params: crate::SlugParams {
                        node_size: Vec2::splat(RENDER_SIZE as f32),
                        layout_flags: 0,
                        alpha_discard: 0.01,
                    },
                    text_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
                    local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
                    ..default()
                });

                let cam = commands
                    .spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())))
                    .id();

                commands.spawn((
                    crate::SlugTextNode {
                        text: text_owned.clone(),
                        font_size: RENDER_SIZE as f32,
                        color: [0, 0, 0, 255],
                        layout: crate::Layout::default(),
                        depth: None,
                    },
                    crate::SlugTextFont(fid),
                    MaterialNode(mat),
                    Node {
                        width: Val::Px(RENDER_SIZE as f32),
                        height: Val::Px(RENDER_SIZE as f32),
                        ..default()
                    },
                    UiTargetCamera(cam),
                ));
            },
        );

        run_and_capture(&mut app, &state)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use inner::{
    CaptureShared, CaptureState, HeadlessCapturePlugin, RENDER_SIZE, RenderedFrame,
    build_atlas_and_run, build_headless_app_base, headless_camera, make_render_target,
    render_plot_ui_material_default, render_plot_ui_material_texture, run_and_capture,
};

#[cfg(all(not(target_arch = "wasm32"), not(feature = "webgl")))]
pub use inner::{render_slug_bevy_material, render_slug_bevy_ui_material};

#[cfg(not(target_arch = "wasm32"))]
pub use inner::{
    render_slug_bevy_material_texture, render_slug_bevy_ui_material_texture, render_slug_fps_like,
};
