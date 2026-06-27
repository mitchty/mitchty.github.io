// Integration tests for flan fullscreen post-process shaders applied over a
// deterministic base checkerboard pattern.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

use flan::post_process::{ActiveShader, AvailableShaders, PostProcessPlugin, PostProcessSettings};
use flan::shaders::ShadersPlugin;
use flan::test::lib::GPU_RENDER_LOCK;
use flan::test::lib::bevy::{
    CaptureShared, CaptureState, HeadlessCapturePlugin, RenderedFrame, build_headless_app_base,
    headless_camera, make_render_target, run_and_capture,
};

/// Build a 256x256 RGBA8 checkerboard image: alternating 32 px black/white tiles.
///
/// This mirrors my shadertoy first shader from https://www.shadertoy.com/view/4fKBWz.
/// It gives every fullscreen shader something deterministic and high-contrast
/// to operate on aka sharp edges, equal black/white area, no lighting variation.
fn make_checkerboard_image() -> Image {
    const SIZE: u32 = 256;
    const TILE: u32 = 32;

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let is_white = ((x / TILE) + (y / TILE)).is_multiple_of(2);
            let v: u8 = if is_white { 255 } else { 0 };
            let idx = ((y * SIZE + x) * 4) as usize;
            // RGBA
            pixels[idx] = v;
            pixels[idx + 1] = v;
            pixels[idx + 2] = v;
            pixels[idx + 3] = 255;
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC;
    image
}

/// Spin up a headless Bevy app exactly as a client of flan should.
fn render_scene(shader_name: &str, intensity: f32) -> Option<RenderedFrame> {
    let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
    let mut app = build_headless_app_base();

    app.add_plugins(ShadersPlugin);
    app.add_plugins(PostProcessPlugin);

    let available = AvailableShaders::default();
    let idx = available
        .shaders
        .iter()
        .position(|s| s.name == shader_name)
        .unwrap_or_else(|| panic!("unknown shader name: {shader_name}"));
    app.insert_resource(ActiveShader { index: idx });

    let render_target = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

    // Wait at least 3 frames so post-process bind groups were setup. Need a
    // better way to approach this.
    app.add_plugins(HeadlessCapturePlugin {
        handle: render_target.clone(),
        state: state.clone(),
        min_frames: 3,
    });

    let rt = render_target.clone();
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mut images: ResMut<Assets<Image>>,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>| {
            let checker_handle = images.add(make_checkerboard_image());

            let mat = materials.add(StandardMaterial {
                base_color_texture: Some(checker_handle),
                // This is setup so the material is consistent, we don't want lighting here.
                unlit: true,
                ..default()
            });

            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(4.0, 4.0))),
                MeshMaterial3d(mat),
                Transform::default(),
            ));

            commands.spawn((
                Camera3d::default(),
                headless_camera(),
                RenderTarget::from(rt.clone()),
                PostProcessSettings {
                    intensity,
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        },
    );

    run_and_capture(&mut app, &state)
}

/// Assert that the shader effect was applied, we don't actually test the output
/// at all for now just that the fullscreen effect changed the base image
/// really. Basically invert the SSIM check and look for non 1.0 SSIM threshhold.
fn assert_effect_applied(shader_name: &str, base: &RenderedFrame, effect: &RenderedFrame) {
    use image::ImageBuffer;

    let base_img: image::RgbaImage =
        ImageBuffer::from_raw(base.width, base.height, base.pixels.clone())
            .expect("base pixel buffer invalid");
    let effect_img: image::RgbaImage =
        ImageBuffer::from_raw(effect.width, effect.height, effect.pixels.clone())
            .expect("effect pixel buffer invalid");

    let result = image_compare::rgba_hybrid_compare(&base_img, &effect_img)
        .unwrap_or_else(|e| panic!("{shader_name}: SSIM compare failed: {e}"));

    assert!(
        result.score < 0.999,
        "{shader_name}: SSIM {:.6} >= 0.999 it appears the fullscreen effect did not modify the scene output.",
        result.score,
    );
}

fn run_fullscreen_test(shader_name: &str) {
    let Some(base) = render_scene(shader_name, 0.0) else {
        return; // no GPU adapter - skip
    };
    let Some(effect) = render_scene(shader_name, 1.0) else {
        return;
    };

    assert_effect_applied(shader_name, &base, &effect);
}

#[test]
fn fullscreen_chromatic_aberration() {
    run_fullscreen_test("chromatic-aberration");
}

#[test]
fn fullscreen_vhs_effect() {
    run_fullscreen_test("vhs-effect");
}

#[test]
fn fullscreen_em_interference() {
    run_fullscreen_test("em-interference");
}

#[test]
fn fullscreen_oil_painting() {
    run_fullscreen_test("oil-painting");
}

#[test]
fn fullscreen_edge_cartoon() {
    run_fullscreen_test("edge-cartoon");
}
