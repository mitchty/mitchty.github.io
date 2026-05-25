// Slug text renderer UiMaterial entry-point shader version
//
// Bevy provides the vertex shader for UiMaterial; it outputs a single UV
// coordinate per fragment covering the node rect [0,1]².
//
// This shader:
//   1. Converts UV to pixel position using slug_params.node_size.
//   2. Calls slugtext() from the slug lib with:
//        - The full node as the draw rect (sub-rect callers set via SlugMaterial).
//        - run_idx = 0 (single-string UiMaterial path).
//   3. Returns slug_params.text_color * coverage.
//
// Color and layout are UiMaterial-local concerns, not slug lib concerns.
// They live in the SlugParams uniform at offsets that the lib never reads:
//   offset 16: text_color vec4<f32>  (applied here at the call site)
//
// For multi-string or custom-layout shaders, import slugtext() directly from
// lib/slug/render.wesl and declare your own bindings in the uber shader using
// this crap.

import package::flan::shaders::lib::input::plot::FragmentInput;
import package::flan::shaders::lib::slug::render::{
    slug_params,
    slugtext,
};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    // Convert UV -> node-space pixels.
    let px        = in.uv * slug_params.node_size;
    let node_size = slug_params.node_size;

    // Full node rect as the draw region.
    let rect = vec4<f32>(0.0, 0.0, node_size.x, node_size.y);

    let color = slug_params.text_color;

    // layout_flags is set per-entity from SlugTextNode.layout via sync_text_meshes.
    let coverage = slugtext(px, rect, slug_params.layout_flags, 0u);

    return color * coverage;
}
