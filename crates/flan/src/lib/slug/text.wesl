// Slug text renderer for full UV space.
//
// Uses the slug text algorithm to render arbitrary text across an entire
// UV space. This is a simplified entry point that centers text and applies
// a fixed single color for the SDF font shapes.

import package::flan::shaders::lib::input::plot::FragmentInput;
import package::flan::shaders::lib::slug::render::slugtext;

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let text_region = vec4<f32>(0.0, 0.0, 1.0, 1.0);
    let coverage = slugtext(in.uv, text_region);

    // TODO: I'll add color in later I forgot I hard coded this to neon green
    // for now. Oops.
    //
    // Green text color (FPS style) with coverage as alpha This
    // can be parameterized later if needed
    let text_color = vec4<f32>(0.0, 1.0, 0.0, coverage);

    return text_color;
}
