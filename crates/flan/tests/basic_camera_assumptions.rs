// Camera setup assumption tests

// Here mostly to track down upgrade to 0.19 issues with the headless renderer.
#![cfg(not(target_arch = "wasm32"))]

use bevy::asset::Handle;
use bevy::camera::{Camera, CameraOutputMode, ClearColor, ClearColorConfig};
use bevy::camera::{NormalizedRenderTarget, RenderTarget};
use bevy::color::Srgba;
use bevy::image::Image;
use bevy::math::UVec2;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::WindowRef;

use flan::test::lib::GPU_RENDER_LOCK;
use flan::test::lib::bevy::{
    CaptureShared, CaptureState, HeadlessCapturePlugin, RENDER_SIZE, build_headless_app_base,
    headless_camera, make_render_target,
};

/// ClearColor::default() bytes is [43, 44, 47, 255].
#[test]
fn world_default_clear_color_is_clear_opaque() {
    let Srgba {
        red,
        green,
        blue,
        alpha,
    } = Srgba::from(ClearColor::default().0);
    let bytes = [
        (red * 255.0).round() as u8,
        (green * 255.0).round() as u8,
        (blue * 255.0).round() as u8,
        (alpha * 255.0).round() as u8,
    ];
    assert_eq!(
        bytes,
        [43, 44, 47, 255],
        "ClearColor::default() must match the clear color"
    );
}

/// Camera::default().clear_color == ClearColorConfig::Default. Without
/// explicitly setting clear_color the camera uses the world ClearColor instead.
#[test]
fn camera_default_clear_color_field_uses_world_resource() {
    assert!(matches!(
        Camera::default().clear_color,
        ClearColorConfig::Default
    ));
}

/// CameraOutputMode::Write.clear_color also defaults to
/// ClearColorConfig::Default. This is here to test/prove/validate that the
/// clear color doesn't show up where I don't want it, mostly on metal.
#[test]
fn camera_output_mode_write_clear_color_also_defaults_to_world_resource() {
    match Camera::default().output_mode {
        CameraOutputMode::Write { clear_color, .. } => {
            assert!(matches!(clear_color, ClearColorConfig::Default));
        }
        other => panic!("expected CameraOutputMode::Write, got {other:?}"),
    }
}

/// RenderTarget::Image normalizes to Some(...) with no primary window setup.
#[test]
fn image_render_target_normalizes_without_primary_window() {
    let rt = RenderTarget::from(Handle::<Image>::default());
    let normalized = rt.normalize(None);
    assert!(
        normalized.is_some(),
        "RenderTarget::Image must return Some() without a primary window setup"
    );
    assert!(matches!(
        normalized.unwrap(),
        NormalizedRenderTarget::Image(_)
    ));
}

/// RenderTarget::Window(Primary) returns None without a primary window.
#[test]
fn window_primary_render_target_returns_none_without_primary_window() {
    let rt = RenderTarget::Window(WindowRef::Primary);
    assert!(
        rt.normalize(None).is_none(),
        "RenderTarget::Window(Primary) must fail to normalize without a primary window setup"
    );
}

/// Camera::default().physical_target_size() is None before camera_system runs.
/// Only camera_system in PostUpdate writes computed.target_info.
#[test]
fn camera_physical_target_size_is_none_before_camera_system_runs() {
    assert!(Camera::default().physical_target_size().is_none());
}

/// make_render_target creates a 256x256 Rgba8UnormSrgb image Asset.
#[test]
fn make_render_target_creates_256x256_rgba8_unorm_srgb_image() {
    let mut images = Assets::<Image>::default();
    let handle = make_render_target(&mut images);
    let image = images
        .get(&handle)
        .expect("image must be in Assets immediately after make_render_target");
    assert_eq!(image.size(), UVec2::new(RENDER_SIZE, RENDER_SIZE));
    assert_eq!(
        image.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb
    );
}

/// vec4(0,0,0,1) in Rgba8UnormSrgb encodes to [0,0,0,255].
/// Here to prove we write to the pattern expected by the unit tests.
#[test]
fn solid_black_linear_encodes_to_0_0_0_255_in_rgba8_unorm_srgb() {
    let bytes: [u8; 4] = [0.0f32, 0.0, 0.0, 1.0].map(|c| (c * 255.0).round() as u8);
    assert_eq!(bytes, [0, 0, 0, 255]);

    // Just in case metal somehow gets weird again.
    assert_ne!([43u8, 44, 47, 255], [0, 0, 0, 255]);
}

/// 256x256x4 = 262144 bytes exactly, just to be sure, some of these tests
/// aren't likely needed.
#[test]
fn render_size_pixel_buffer_is_262144_bytes() {
    let n = RENDER_SIZE as usize;
    assert_eq!(n * n * 4, 262144);
}

#[test]
fn startup_postupdate_run_before_extract_schedule_on_first_tick() {
    use std::sync::{Arc, Mutex};

    let order_log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let log_startup = order_log.clone();
    let log_postupdate = order_log.clone();

    let mut app = build_headless_app_base();

    app.add_systems(Startup, move || {
        log_startup.lock().unwrap().push("startup");
    });
    app.add_systems(PostUpdate, move || {
        log_postupdate.lock().unwrap().push("postupdate");
    });

    app.finish();
    app.cleanup();
    app.update(); // one full tick

    let log = order_log.lock().unwrap();
    let startup_pos = log.iter().position(|&s| s == "startup");
    let postupdate_pos = log.iter().position(|&s| s == "postupdate");
    assert!(startup_pos.is_some(), "Startup must fire on first tick");
    assert!(
        postupdate_pos.is_some(),
        "PostUpdate must fire on first tick"
    );
    assert!(
        startup_pos.unwrap() < postupdate_pos.unwrap(),
        "Startup must fire before PostUpdate and if this fails, camera_system in PostUpdate cannot see the image added in Startup"
    );
}

#[test]
fn camera_system_populates_target_info_for_image_render_target() {
    use std::sync::{Arc, Mutex};

    let observed: Arc<Mutex<Vec<Option<UVec2>>>> = Arc::new(Mutex::new(Vec::new()));
    let observed_clone = observed.clone();

    let mut app = build_headless_app_base();

    let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

    let ih = image_handle.clone();
    app.add_systems(Startup, move |mut commands: Commands| {
        commands.spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())));
    });

    let obs = observed_clone.clone();
    app.add_systems(PostUpdate, move |cameras: Query<&Camera>| {
        for cam in &cameras {
            obs.lock().unwrap().push(cam.physical_target_size());
        }
    });

    app.finish();
    app.cleanup();

    let mut target_size_observed = None;
    for _ in 0..5 {
        app.update();
        let readings = observed.lock().unwrap();
        if let Some(&Some(sz)) = readings.iter().find(|r| r.is_some()) {
            target_size_observed = Some(sz);
            break;
        }
    }

    let readings = observed.lock().unwrap();
    if readings.is_empty() {
        eprintln!(
            "camera_system_populates_target_info: no camera readings - likely no GPU adapter, skipping"
        );
        return;
    }

    assert_eq!(
        target_size_observed,
        Some(UVec2::new(RENDER_SIZE, RENDER_SIZE)),
        "camera.physical_target_size() must return Some({RENDER_SIZE}x{RENDER_SIZE}) after the camera_system runs. Instead got: {readings:?}\n If None the camera_system did not set computed.target_info so check that the image is in Assets<Image> before the camera_system runs in PostUpdate.\n If Some((0,0)): image has zero size  check make_render_target.\nA failing test here means the canary will always see clear color/[43,44,47,255]."
    );
}

fn run_capture_n_frames(app: &mut App, state: &CaptureShared, max_frames: u32) -> Option<Vec<u8>> {
    app.finish();
    app.cleanup();
    for _ in 0..max_frames {
        app.update();
        match &*state.lock().expect("state mutex poisoned") {
            CaptureState::Done(_) | CaptureState::Error(_) => break,
            CaptureState::Pending => {}
        }
    }
    match std::mem::replace(
        &mut *state.lock().expect("state mutex poisoned"),
        CaptureState::Pending,
    ) {
        CaptureState::Done(pixels) => Some(pixels),
        CaptureState::Error(e) if e.contains("no wgpu adapter found") => {
            eprintln!("bisect: skipping - no GPU adapter: {e}");
            None
        }
        CaptureState::Error(e) => panic!("capture error: {e}"),
        CaptureState::Pending => None, // no alpha > 0 observed within max_frames
    }
}

#[test]
fn bisect_layer0_base_app_camera_only_no_ui() {
    use std::sync::{Arc, Mutex};
    let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));

    let mut app = build_headless_app_base();
    let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

    app.add_plugins(HeadlessCapturePlugin {
        handle: image_handle.clone(),
        state: state.clone(),
        min_frames: 0,
    });

    let ih = image_handle.clone();
    app.add_systems(Startup, move |mut commands: Commands| {
        commands.spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())));
    });

    let result = run_capture_n_frames(&mut app, &state, 30);
    eprintln!(
        "bisect_layer0: {:?}",
        result.as_ref().map(|p| {
            let sample = &p[..4.min(p.len())];
            format!("[{},{},{},{}]", sample[0], sample[1], sample[2], sample[3])
        })
    );
    // Document result rather than assert this is what I saw on vulkan but not metal.
    // Iff pixels = [43,44,47,255] then output-write clear applies world color even with no UI pass.
    // Iff None nothing produced alpha > 0 and the camera truly cleared transparently.
}

/// Verify that adding ShadersPlugin alone doesn't change the clear behavior.
#[test]
fn bisect_layer1_base_plus_shaders_plugin_no_ui_material() {
    use std::sync::{Arc, Mutex};
    let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));

    let mut app = build_headless_app_base();
    app.add_plugins(flan::shaders::ShadersPlugin);

    let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());
    app.add_plugins(HeadlessCapturePlugin {
        handle: image_handle.clone(),
        state: state.clone(),
        min_frames: 0,
    });

    let ih = image_handle.clone();
    app.add_systems(Startup, move |mut commands: Commands| {
        commands.spawn((Camera2d, headless_camera(), RenderTarget::from(ih.clone())));
    });

    let result = run_capture_n_frames(&mut app, &state, 30);
    eprintln!(
        "bisect_layer1: {:?}",
        result.as_ref().map(|p| {
            let s = &p[..4.min(p.len())];
            format!("[{},{},{},{}]", s[0], s[1], s[2], s[3])
        })
    );
}

/// This is identical to the canary test render test. CanaryMaterial outputs black
/// (vec4(0,0,0,1)) unconditionally. If this renders correctly, all pixels must
/// be [0,0,0,255].
///
/// If this produces [43,44,47,255] but layers 0 and 1 produce a different
/// value, then adding UiMaterialPlugin<CanaryMaterial> is the breaking change.
///
/// If however all three layers produce [43,44,47,255], the issue is in the base
/// setup where output-write clear color overwrites everything regardless of the
/// UI pass.
#[test]
fn bisect_layer2_full_canary_same_as_render_canary_test() {
    use std::sync::{Arc, Mutex};
    let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));

    let mut app = build_headless_app_base();
    app.add_plugins(flan::shaders::ShadersPlugin);
    app.add_plugins(UiMaterialPlugin::<flan::test::lib::canary::CanaryMaterial>::default());

    let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());
    app.add_plugins(HeadlessCapturePlugin {
        handle: image_handle.clone(),
        state: state.clone(),
        min_frames: 0,
    });

    let ih = image_handle.clone();
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mut materials: ResMut<Assets<flan::test::lib::canary::CanaryMaterial>>| {
            let mat = materials.add(flan::test::lib::canary::CanaryMaterial {});
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

    let result = run_capture_n_frames(&mut app, &state, 60);
    let first_pixel = result
        .as_ref()
        .map(|p| [p[0], p[1], p[2], p[3]])
        .unwrap_or([0, 0, 0, 0]);
    eprintln!("bisect_layer2: first pixel = {first_pixel:?}");

    let Some(pixels) = result else {
        eprintln!("bisect_layer2: no non-transparent pixels for 60 frames, skipping test");
        return;
    };

    let bad: Vec<(u32, u32, [u8; 4])> = pixels
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            let px = [p[0], p[1], p[2], p[3]];
            if px != [0, 0, 0, 255] {
                Some(((i as u32) % RENDER_SIZE, (i as u32) / RENDER_SIZE, px))
            } else {
                None
            }
        })
        .collect();

    if !bad.is_empty() {
        let first5: Vec<_> = bad.iter().take(5).collect();
        panic!(
            "bisect_layer2: {}/{} pixels are not opaque black\n\
             first bad pixel: {first5:#?}\n\
             If these are still clear color [43,44,47,255] the UiMaterial pass got skipped and output-write used world clear instead.\n\
             Compare with bisect_layer0/1 to see if ShadersPlugin or UiMaterialPlugin caused it or not.",
            bad.len(),
            RENDER_SIZE as usize * RENDER_SIZE as usize,
        );
    }
}
