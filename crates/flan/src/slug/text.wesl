// Slug text renderer - entry-point shader.
//
// Vertex and fragment shaders for per-glyph quad rendering.
//
// Each glyph is a CPU-shaped quad (4 vertices, 6 indices). The vertex shader
// interpolates em-space coordinates across the quad surface. The fragment
// shader evaluates Slug coverage entirely from those interpolated coordinates
// plus the flat glyph index.
//
// Vertex layout (28 bytes, stride 28):
//   @location(0) pos   vec4<f32>  [screen_x, screen_y, sign_x, sign_y]
//   @location(1) glyph vec2<u32>  [pack_i16(em_x, em_y), glyph_index]
//   @location(2) color vec4<f32>  decoded from RGBA8 vertex color

import package::flan::shaders::lib::slug::render::{
    slug_fetch_band_ranges,
    slug_render,
};

struct VertexInput {
    @location(0) pos:   vec4<f32>,
    @location(1) glyph: vec2<u32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position)              clip_pos:    vec4<f32>,
    @location(0)                    color:       vec4<f32>,
    @location(1)                    em_pos:      vec2<f32>,
    @location(2) @interpolate(flat) glyph_index: u32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // pos.xy is already in clip space for UiMaterial / Material2d.
    // pos.zw holds corner signs (reserved for future use).
    out.clip_pos = vec4<f32>(in.pos.xy, 0.0, 1.0);
    out.color    = in.color;

    // Unpack em-space coordinates: high 16 bits = em_x, low 16 bits = em_y.
    // Avoid extractBits / bitfieldExtract entirely - some WebGL2 implementations
    // don't support it. Use plain shifts instead:
    //   high 16 bits [16:32]: arithmetic right-shift the reinterpreted i32 by 16
    //   low  16 bits [0:16]:  shift the u32 left 16 to put the sign bit at MSB,
    //                         reinterpret as i32, then arithmetic right-shift back
    let packed   = in.glyph.x;
    let em_x_i16 = i32(packed) >> 16u;
    let em_y_i16 = i32(packed << 16u) >> 16u;
    out.em_pos      = vec2<f32>(f32(em_x_i16), f32(em_y_i16));
    out.glyph_index = in.glyph.y;

    return out;
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;

    // Compute pixels_per_em HERE in uniform control flow - outside any loop or
    // conditional - so that fwidth() is well-defined under WebGL GLSL ES rules.
    let ems_per_pixel  = fwidth(in.em_pos);
    let pixels_per_em  = vec2<f32>(
        select(0.0, 1.0 / ems_per_pixel.x, ems_per_pixel.x > 1e-10),
        select(0.0, 1.0 / ems_per_pixel.y, ems_per_pixel.y > 1e-10),
    );

    let band_ranges = slug_fetch_band_ranges(in.em_pos, in.glyph_index);
    let coverage    = slug_render(in.em_pos, band_ranges, pixels_per_em);

    out.color = in.color * coverage;
    return out;
}
