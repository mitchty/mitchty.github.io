// Test stuff is weird and not all code paths have live code.
#![allow(dead_code)]

pub mod bevy;
pub mod camera_assumptions;
pub mod canary;
pub mod shader;
pub mod slug_text;
pub mod snapshot;
pub mod stats_overlay;

#[cfg(not(target_arch = "wasm32"))]
pub use bevy::{RENDER_SIZE, RenderedFrame};
#[cfg(not(target_arch = "wasm32"))]
pub use snapshot::{
    DEFAULT_SSIM_THRESHOLD, assert_snapshot, assert_variant_matches_canonical, frame_to_image,
    has_non_background_pixels,
};

/// Number of test points per frame.
pub const TEST_POINT_COUNT: usize = 200;

/// 200-point sin wave in \[0,1\] UV space canonical test signal for image output validation.
pub fn test_sin_wave_points() -> Vec<[f32; 2]> {
    (0..TEST_POINT_COUNT)
        .map(|i| {
            let x = i as f32 / (TEST_POINT_COUNT - 1) as f32;
            let y = (x * 10.0_f32).sin() * 0.4 + 0.5;
            [x, y]
        })
        .collect()
}

/// Single mutex that serializes every headless GPU operation in the test suite.
///
/// All headless Bevy renders acquire this lock before touching the gpu.
/// This prevents concurrent device creation and draw submissions from different
/// test threads from racing on the same driver resources.
#[cfg(not(target_arch = "wasm32"))]
pub static GPU_RENDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `true` when the wgpu error indicates no gpu adapter so skip, don't fail the test.
pub fn has_no_adapter(err: &str) -> bool {
    err.contains("no wgpu adapter found")
}
