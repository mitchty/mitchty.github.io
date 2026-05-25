// Integration test: Slug text renderer - renders "Hello world!" to a 256x256
// RGBA image with a white background and black text, then validates that:
//
//   1. The corner pixel (background) is near-white (coverage ≈ 0).
//   2. At least some pixels near the center have non-zero coverage (text was
//      actually rendered).
//   3. The output PNG is saved to tests/fixtures/slug_hello_world.png and
//      compared against the reference on subsequent runs (snapshot test).
//
// TODO: This entire test file is built on the old pre-new-API legacy functions
// (params_to_bytes, glyphs_to_bytes, bands_to_bytes, etc.) and the old
// LegacySlugAtlas field layout. It needs to be rewritten against the new
// SlugAtlas / slugtext() / SlugRunDesc / SlugDrawData API introduced when
// layout and color moved to the shader call site. Disabled until then.
#![cfg(any())] // never compiled - remove this line when the rewrite is done

use flan::render::{Binding, BindingKind, RENDER_SIZE, render_shader, render_shader_sized};
use flan::slug::{
    DEFAULT_BAND_COUNT, bands_to_bytes, build_slug_atlas, char_advances_to_bytes,
    char_indices_to_bytes, curves_to_bytes, glyphs_to_bytes, params_to_bytes,
};
use flan::snapshot::{DEFAULT_SSIM_THRESHOLD, assert_snapshot};
use flan::wesl::{Variant, compile};

// TODO: Need to figure out all the font data at some point. Embedding it is/was
// fine but its getting crazy. I need to convert this all to Bevy directly.
const FIRA_TTF: &[u8] = include_bytes!("fixtures/FiraMono-Medium.ttf");
const HELLO: &str = "Hello world!";

// Test entry wesl shader.
//
// This is the entry point compiled by the test harness. It imports the
// "real" lib/slug/render.wesl slugtext() function so the test exercises
// exactly the same shader code that runs in the Bevy app, compiled via the
// same wesl::compile() path used by all other integration tests.
const SLUG_TEST_WESL: &str = r#"
import package::flan::shaders::lib::input::plot::FragmentInput;
import package::flan::shaders::lib::slug::render::slugtext;

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let text_region = vec4<f32>(0.0, 0.0, 1.0, 1.0);
    let coverage    = slugtext(in.uv, text_region);
    // White background, black text.
    let bg   = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    let text = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    return mix(bg, text, coverage);
}
"#;

// Compile the test WESL to WGSL using the production lib modules as:
// (group 0, WGPU_TEST variant so @if(!UI_MATERIAL) bindings are selected).
fn compile_slug_wgsl() -> String {
    compile("slug/text-test", SLUG_TEST_WESL, Variant::TEST_MATERIAL)
        .unwrap_or_else(|e| panic!("WESL compile failed for slug test shader: {e}"))
}

// Build a variant of the compiled WGSL with a custom text_region by replacing
// the sentinel line that the test entry shader emits.
fn slug_wgsl_with_region(x0: f32, y0: f32, x1: f32, y1: f32) -> String {
    compile_slug_wgsl().replace(
        "let text_region = vec4<f32>(0.0, 0.0, 1.0, 1.0);",
        &format!("let text_region = vec4<f32>({x0}, {y0}, {x1}, {y1});"),
    )
}

/// Render `text` into a sub-region of the UV square specified by
/// `(x0, y0) .. (x1, y1)` in [0,1]^2 uv space.
fn render_text_region(
    text: &str,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> Option<flan::render::RenderedFrame> {
    let atlas = build_slug_atlas(FIRA_TTF, text, DEFAULT_BAND_COUNT)
        .unwrap_or_else(|e| panic!("atlas build failed for {text:?}: {e}"));

    let params_bytes = params_to_bytes(
        atlas.glyphs.len() as u32,
        atlas.char_glyph_indices.len() as u32,
        0,
        1.0,
        atlas.font_y_min,
        atlas.font_y_max,
    );
    let glyphs_bytes = glyphs_to_bytes(&atlas.glyphs);
    let curves_bytes = curves_to_bytes(&atlas.curves);
    let bands_bytes = bands_to_bytes(&atlas.bands);
    let chars_bytes = char_indices_to_bytes(&atlas.char_glyph_indices);
    let advances_bytes = char_advances_to_bytes(&atlas.char_advances);

    let wgsl = slug_wgsl_with_region(x0, y0, x1, y1);

    let result = render_shader(
        &wgsl,
        &[
            Binding {
                slot: 0,
                kind: BindingKind::Uniform,
                data: &params_bytes,
            },
            Binding {
                slot: 1,
                kind: BindingKind::StorageRead,
                data: &glyphs_bytes,
            },
            Binding {
                slot: 2,
                kind: BindingKind::StorageRead,
                data: &curves_bytes,
            },
            Binding {
                slot: 3,
                kind: BindingKind::StorageRead,
                data: &bands_bytes,
            },
            Binding {
                slot: 4,
                kind: BindingKind::StorageRead,
                data: &chars_bytes,
            },
            Binding {
                slot: 5,
                kind: BindingKind::StorageRead,
                data: &advances_bytes,
            },
        ],
    );

    match result {
        Ok(f) => Some(f),
        Err(ref e) if has_no_adapter(e) => {
            eprintln!("skipping region render of {text:?}: no GPU adapter found");
            None
        }
        Err(e) => panic!("render_shader (region) failed for {text:?}: {e}"),
    }
}

fn has_no_adapter(e: &str) -> bool {
    e.contains("no wgpu adapter found")
}

// Shared render helper
//
// Builds a SlugAtlas for input `text`, serializes it, runs the headless GPU
// render, and returns the RenderedFrame. Returns None if no GPU adapter is
// present. Panics on any other error.
fn render_text(text: &str) -> Option<flan::render::RenderedFrame> {
    let atlas = build_slug_atlas(FIRA_TTF, text, DEFAULT_BAND_COUNT)
        .unwrap_or_else(|e| panic!("atlas build failed for {text:?}: {e}"));

    let params_bytes = params_to_bytes(
        atlas.glyphs.len() as u32,
        atlas.char_glyph_indices.len() as u32,
        0,
        1.0,
        atlas.font_y_min,
        atlas.font_y_max,
    );
    let glyphs_bytes = glyphs_to_bytes(&atlas.glyphs);
    let curves_bytes = curves_to_bytes(&atlas.curves);
    let bands_bytes = bands_to_bytes(&atlas.bands);
    let chars_bytes = char_indices_to_bytes(&atlas.char_glyph_indices);
    let advances_bytes = char_advances_to_bytes(&atlas.char_advances);

    let wgsl = compile_slug_wgsl();

    let result = render_shader(
        &wgsl,
        &[
            Binding {
                slot: 0,
                kind: BindingKind::Uniform,
                data: &params_bytes,
            },
            Binding {
                slot: 1,
                kind: BindingKind::StorageRead,
                data: &glyphs_bytes,
            },
            Binding {
                slot: 2,
                kind: BindingKind::StorageRead,
                data: &curves_bytes,
            },
            Binding {
                slot: 3,
                kind: BindingKind::StorageRead,
                data: &bands_bytes,
            },
            Binding {
                slot: 4,
                kind: BindingKind::StorageRead,
                data: &chars_bytes,
            },
            Binding {
                slot: 5,
                kind: BindingKind::StorageRead,
                data: &advances_bytes,
            },
        ],
    );

    match result {
        Ok(f) => Some(f),
        Err(ref e) if has_no_adapter(e) => {
            eprintln!("skipping slug render of {text:?}: no GPU adapter found");
            None
        }
        Err(e) => panic!("render_shader failed for {text:?}: {e}"),
    }
}

// Check that all four corners are near-white background, aka not covered by
// glyphs.
fn assert_corners_white(frame: &flan::render::RenderedFrame) {
    let w = frame.width - 1;
    let h = frame.height - 1;
    for (cx, cy) in [(0, 0), (w, 0), (0, h), (w, h)] {
        let px = pixel_rgba(frame, cx, cy);
        assert!(
            px[0] > 200 && px[1] > 200 && px[2] > 200,
            "corner ({cx},{cy}) should be near-white background, got {px:?}",
        );
    }
}

// Check that at least one dark pixel exists in the middle third of the image,
// proving that some text coverage was actually rendered.
fn assert_has_dark_pixels(frame: &flan::render::RenderedFrame, label: &str) {
    let h = frame.height;
    let w = frame.width;
    let margin = h / 3;
    let mut found = false;
    'outer: for row in margin..(h - margin) {
        for col in 0..w {
            let px = pixel_rgba(frame, col, row);
            if px[0] < 200 || px[1] < 200 || px[2] < 200 {
                found = true;
                break 'outer;
            }
        }
    }
    assert!(
        found,
        "{label}: no dark pixels in central rows - nothing was rendered"
    );
}

// Single character 'a'
//
// Simplest possible use case: one glyph, one character instance.
// Validates that the atlas builder and shader agree on curve/band data for a
// single lowercase 'a' before we test anything more complex.
#[test]
fn render_single_char_a() {
    let Some(frame) = render_text("a") else {
        return;
    };

    assert_eq!(frame.width, RENDER_SIZE);
    assert_eq!(frame.height, RENDER_SIZE);

    assert_corners_white(&frame);
    assert_has_dark_pixels(&frame, "single char 'a'");

    assert_snapshot("slug_char_a", &frame, DEFAULT_SSIM_THRESHOLD);
}

// Test " a" leading space then 'a' output.
//
// Ensures that a leading no-outline glyph or space advances the cursor cleanly
// without polluting the coverage of the following 'a'. The rendered output
// must look identical to the plain 'a' render except shifted right by one
// space-width. All four corners must be white and text must be present.
#[test]
fn render_space_then_a() {
    let Some(frame) = render_text(" a") else {
        return;
    };

    assert_eq!(frame.width, RENDER_SIZE);
    assert_eq!(frame.height, RENDER_SIZE);

    assert_corners_white(&frame);
    assert_has_dark_pixels(&frame, "' a' (space + a)");

    // The ' a' render should have no dark pixels in the left quarter of the
    // image and the space pushes the 'a' to the right, so the left side of the
    // region should remain white background.
    let quarter = frame.width / 4;
    for col in 0..quarter {
        for row in 0..frame.height {
            let px = pixel_rgba(&frame, col, row);
            assert!(
                px[0] > 200 && px[1] > 200 && px[2] > 200,
                "left quarter should be white (space region), got {px:?} at ({col},{row})",
            );
        }
    }

    assert_snapshot("slug_space_then_a", &frame, DEFAULT_SSIM_THRESHOLD);
}

// Test "Hello world!" string across the full UV region
#[test]
fn render_hello_world_centered() {
    let Some(frame) = render_text(HELLO) else {
        return;
    };

    assert_eq!(frame.width, RENDER_SIZE);
    assert_eq!(frame.height, RENDER_SIZE);

    assert_corners_white(&frame);
    assert_has_dark_pixels(&frame, HELLO);

    assert_snapshot("slug_hello_world", &frame, DEFAULT_SSIM_THRESHOLD);
}

// Test "Hello world!" rendered into the bottom third of the UV space.
//
// text_region = (0.0, 2/3, 1.0, 1.0) to the lower third of the image.
#[test]
fn render_hello_world_bottom_third() {
    // y0 = 2/3, y1 = 1.0
    let y0: f32 = 2.0 / 3.0;
    let Some(frame) = render_text_region(HELLO, 0.0, y0, 1.0, 1.0) else {
        return;
    };

    let w = frame.width;
    let h = frame.height;
    assert_eq!(w, RENDER_SIZE);
    assert_eq!(h, RENDER_SIZE);

    assert_corners_white(&frame);

    // Top two-thirds must be completely white no text rendered above the region.
    let boundary_row = (y0 * h as f32) as u32;
    for row in 0..boundary_row {
        for col in 0..w {
            let px = pixel_rgba(&frame, col, row);
            assert!(
                px[0] > 200 && px[1] > 200 && px[2] > 200,
                "pixel ({col},{row}) is above the text_region and should be white, got {px:?}",
            );
        }
    }

    // Bottom third must contain at least one dark pixel which should roughly be
    // the text itself.
    let mut found_dark = false;
    'outer: for row in boundary_row..h {
        for col in 0..w {
            let px = pixel_rgba(&frame, col, row);
            if px[0] < 200 || px[1] < 200 || px[2] < 200 {
                found_dark = true;
                break 'outer;
            }
        }
    }
    assert!(
        found_dark,
        "no dark pixels in the bottom third text was not rendered in the region"
    );

    assert_snapshot(
        "slug_hello_world_bottom_third",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
    );
}

// Test: "Hello world!" in the top third, centered horizontally at 50% width.
//
// text_region = (0.25, 0.0, 0.75, 1/3)
#[test]
fn render_hello_world_top_third_center_half() {
    let x0: f32 = 0.25;
    let x1: f32 = 0.75;
    let y1: f32 = 1.0 / 3.0;
    let Some(frame) = render_text_region(HELLO, x0, 0.0, x1, y1) else {
        return;
    };

    let w = frame.width;
    let h = frame.height;
    assert_eq!(w, RENDER_SIZE);
    assert_eq!(h, RENDER_SIZE);

    assert_corners_white(&frame);

    let x_lo = (x0 * w as f32) as u32;
    let x_hi = (x1 * w as f32) as u32;
    let y_boundary = (y1 * h as f32) as u32;

    // Left quarter must be white outside the region.
    for col in 0..x_lo {
        for row in 0..h {
            let px = pixel_rgba(&frame, col, row);
            assert!(
                px[0] > 200 && px[1] > 200 && px[2] > 200,
                "pixel ({col},{row}) is left of the text_region and should be white, got {px:?}",
            );
        }
    }

    // Right quarter must be white outside the region.
    for col in x_hi..w {
        for row in 0..h {
            let px = pixel_rgba(&frame, col, row);
            assert!(
                px[0] > 200 && px[1] > 200 && px[2] > 200,
                "pixel ({col},{row}) is right of the text_region and should be white, got {px:?}",
            );
        }
    }

    // Bottom two-thirds must be white outside the region.
    for row in y_boundary..h {
        for col in 0..w {
            let px = pixel_rgba(&frame, col, row);
            assert!(
                px[0] > 200 && px[1] > 200 && px[2] > 200,
                "pixel ({col},{row}) is below the text_region and should be white, got {px:?}",
            );
        }
    }

    // Inside the region there must be at least one dark pixel.
    let mut found_dark = false;
    'outer: for row in 0..y_boundary {
        for col in x_lo..x_hi {
            let px = pixel_rgba(&frame, col, row);
            if px[0] < 200 || px[1] < 200 || px[2] < 200 {
                found_dark = true;
                break 'outer;
            }
        }
    }
    assert!(
        found_dark,
        "no dark pixels inside the top-third/center-half region - text was not rendered"
    );

    assert_snapshot(
        "slug_hello_world_top_third_center_half",
        &frame,
        DEFAULT_SSIM_THRESHOLD,
    );
}

fn pixel_rgba(frame: &flan::render::RenderedFrame, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * frame.width + x) * 4) as usize;
    [
        frame.pixels[offset],
        frame.pixels[offset + 1],
        frame.pixels[offset + 2],
        frame.pixels[offset + 3],
    ]
}

// Helper: render a text string into a width x height rectangle using the
// fill-height left-aligned, with no letterbox slug layout.
fn render_text_rect(text: &str, width: u32, height: u32) -> Option<flan::render::RenderedFrame> {
    let atlas = build_slug_atlas(FIRA_TTF, text, DEFAULT_BAND_COUNT)
        .unwrap_or_else(|e| panic!("atlas build failed for {text:?}: {e}"));

    let text_layout: u32 = 0x04;
    let node_aspect = width as f32 / height as f32;
    let params_bytes = params_to_bytes(
        atlas.glyphs.len() as u32,
        atlas.char_glyph_indices.len() as u32,
        text_layout,
        node_aspect,
        atlas.font_y_min,
        atlas.font_y_max,
    );
    let glyphs_bytes = glyphs_to_bytes(&atlas.glyphs);
    let curves_bytes = curves_to_bytes(&atlas.curves);
    let bands_bytes = bands_to_bytes(&atlas.bands);
    let chars_bytes = char_indices_to_bytes(&atlas.char_glyph_indices);
    let advances_bytes = char_advances_to_bytes(&atlas.char_advances);

    let wgsl = compile_slug_wgsl();

    let result = render_shader_sized(
        width,
        height,
        &wgsl,
        &[
            Binding {
                slot: 0,
                kind: BindingKind::Uniform,
                data: &params_bytes,
            },
            Binding {
                slot: 1,
                kind: BindingKind::StorageRead,
                data: &glyphs_bytes,
            },
            Binding {
                slot: 2,
                kind: BindingKind::StorageRead,
                data: &curves_bytes,
            },
            Binding {
                slot: 3,
                kind: BindingKind::StorageRead,
                data: &bands_bytes,
            },
            Binding {
                slot: 4,
                kind: BindingKind::StorageRead,
                data: &chars_bytes,
            },
            Binding {
                slot: 5,
                kind: BindingKind::StorageRead,
                data: &advances_bytes,
            },
        ],
    );

    match result {
        Ok(f) => Some(f),
        Err(ref e) if e.contains("no wgpu adapter found") => {
            eprintln!("skipping rect render of {text:?}: no GPU adapter found");
            None
        }
        Err(e) => panic!("render_shader_sized failed for {text:?}: {e}"),
    }
}

// Test a rectangular viewport 8:1 aspect ratio (512 x 64).
//
// Validates that the fill-height slug renderer fills the full vertical extent
// of a wide rectangular node - matching how a UI label would be sized.
#[test]
fn render_rect_fill_height() {
    // 8:1 aspect ratio - wide label, short height.
    let width: u32 = 512;
    let height: u32 = 64;

    let Some(frame) = render_text_rect(HELLO, width, height) else {
        return;
    };

    assert_eq!(frame.width, width);
    assert_eq!(frame.height, height);

    // Corners should be background color of white.
    let w = frame.width - 1;
    let h = frame.height - 1;
    for (cx, cy) in [(0, 0), (w, 0), (0, h), (w, h)] {
        let px = pixel_rgba(&frame, cx, cy);
        assert!(
            px[0] > 200 && px[1] > 200 && px[2] > 200,
            "corner ({cx},{cy}) should be near-white background, got {px:?}",
        );
    }

    // At least one dark pixel must exist to act as the text was rendered.
    let mut found_dark = false;
    'outer: for row in 0..frame.height {
        for col in 0..frame.width {
            let px = pixel_rgba(&frame, col, row);
            if px[0] < 200 || px[1] < 200 || px[2] < 200 {
                found_dark = true;
                break 'outer;
            }
        }
    }
    assert!(
        found_dark,
        "render_rect_fill_height: no dark pixels - text was not rendered"
    );

    // No row should be entirely white glyphs must reach both the top and
    // bottom of the image since the text fills the full height.
    let col_lo = frame.width / 10;
    let col_hi = frame.width - col_lo;
    let mut blank_rows: Vec<u32> = Vec::new();
    for row in 0..frame.height {
        let all_white = (col_lo..col_hi).all(|col| {
            let px = pixel_rgba(&frame, col, row);
            px[0] > 200 && px[1] > 200 && px[2] > 200
        });
        if all_white {
            blank_rows.push(row);
        }
    }
    assert!(
        blank_rows.is_empty(),
        "render_rect_fill_height: rows {:?} are entirely white - text is NOT filling the full height (letterboxed)",
        &blank_rows[..blank_rows.len().min(5)],
    );

    assert_snapshot("slug_rect_fill_height", &frame, DEFAULT_SSIM_THRESHOLD);
}
