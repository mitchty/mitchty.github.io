// Slug text renderer 3d world-space shader.
//
// This is the Material3d shader entry point.
//
// The cpu pre-computes local-to-clip matrix (projection * view * model) each
// frame and stores it in the material uniform.
//
// Coordinate convention is the mesh produced by flan is normalized so the full text run is 1 world unit tall.
//
// Vertex layout:
//   @location(0) pos   vec4<f32>  - local-space xy quad corner (z/w reserved)
//   @location(1) glyph vec2<u32>  - [pack_i16(em_x, em_y), glyph_index]
//   @location(2) color vec4<f32>  - decoded RGBA8 vertex color

import package::flan::shaders::lib::slug::render::{
    slug_fetch_band_ranges,
    slug_render,
};

// Extended 96-byte binding 0 for the 3d material path.
//
// Layout:
//   offset  0: node_size     vec2<f32>     unused
//   offset  8: layout_flags  u32           unused
//   offset 12: _pad          u32
//   offset 16: text_color    vec4<f32>     RGBA linear, applied to coverage
//   offset 32: local_to_clip mat4x4<f32>   projection * view * model
struct SlugMesh3dParams {
    node_size:     vec2<f32>,
    layout_flags:  u32,
    _pad:          u32,
    text_color:    vec4<f32>,
    local_to_clip: mat4x4<f32>,
}

// Bevy 0.18+? uses MATERIAL_BIND_GROUP_INDEX = 3 for the 3-D Material pipeline.
@group(3) @binding(0) var<uniform> slug_3d_params: SlugMesh3dParams;

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

    // Transform from local model space to clip space using pre-computed
    // matrix. in.pos.z carries the local-space depth offset: 0.0 for flat
    // text, +/- the half_depth for extruded front/back faces.
    out.clip_pos = slug_3d_params.local_to_clip * vec4<f32>(in.pos.xy, in.pos.z, 1.0);

    // Use material text_color rather than per vertex color so changing
    // SlugTextNode.color reflects immediately without a mesh rebuild.
    out.color = slug_3d_params.text_color;

    // Unpack i16 em-space coordinates from the packed u32 (same as text.wesl).
    let packed   = in.glyph.x;
    let em_x_i16 = i32(packed) >> 16u;
    let em_y_i16 = i32(packed << 16u) >> 16u;
    out.em_pos      = vec2<f32>(f32(em_x_i16), f32(em_y_i16));
    out.glyph_index = in.glyph.y;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // fwidth must be called outside any branch or loop for uniform control
    // flow.
    let ems_per_pixel = fwidth(in.em_pos);
    let pixels_per_em = vec2<f32>(
        select(0.0, 1.0 / ems_per_pixel.x, ems_per_pixel.x > 1e-10),
        select(0.0, 1.0 / ems_per_pixel.y, ems_per_pixel.y > 1e-10),
    );

    let band_ranges = slug_fetch_band_ranges(in.em_pos, in.glyph_index);
    let coverage    = slug_render(in.em_pos, band_ranges, pixels_per_em);

    return in.color * coverage;
}
