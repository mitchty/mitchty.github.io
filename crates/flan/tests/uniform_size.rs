//! Temporary? tests for uniform nonsense I never knew about with metal about 16
//! byte shader data. I didn't know that say you have 40 bytes of uniform data,
//! you need to pass in 48 bytes of data in metal via wgpu. Bevy does this for
//! us but with my hack wgpu test setup its not great. Note its not padding it
//! just needs to be a multiple of 16.

use flan::wesl::{Variant, compile};

#[repr(C)]
#[allow(dead_code)]
struct PlotUniform {
    min: [f32; 2],
    max: [f32; 2],
    zoom: [f32; 2],
    offset: [f32; 2],
    count: u32,
    time: f32,
    line_width: f32,
    _padding: f32,
}

const PLOT_WESL: &str = include_str!("../src/2d/plot.wesl");

#[test]
fn plot_uniform_rust_wgsl_size_match() {
    use std::mem::size_of;

    let wgsl = compile("2d/plot", PLOT_WESL, Variant::TEST_MATERIAL)
        .expect("Failed to compile WESL shader");

    let plot_uniform_lines: Vec<&str> = wgsl
        .lines()
        .skip_while(|l| !l.contains("struct PlotUniform {"))
        .skip(1) // Skip the opening brace
        .take_while(|l| !l.contains('}'))
        .collect();

    let mut wgsl_size = 0;
    for line in &plot_uniform_lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if line.contains("vec2<f32>") {
            wgsl_size += 8;
        } else if line.contains("f32") || line.contains("u32") {
            wgsl_size += 4;
        }
    }

    // The Rust struct should be >= WGSL size we add padding for Metal alignment transparently ish
    assert!(
        size_of::<PlotUniform>() >= wgsl_size,
        "Rust PlotUniform ({} bytes) must be >= WGSL fields size ({} bytes). \
         Rust: {:?}, WGSL: {}",
        size_of::<PlotUniform>(),
        wgsl_size,
        plot_uniform_lines,
        wgsl,
    );

    // And it should be a multiple of 16 for Metal alignment
    assert_eq!(
        size_of::<PlotUniform>() % 16,
        0,
        "Rust PlotUniform size ({} bytes) must be a multiple of 16 for Metal alignment",
        size_of::<PlotUniform>()
    );
}
