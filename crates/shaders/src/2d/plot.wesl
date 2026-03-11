import package::shaders::lib::helpers::plot::{draw_line, draw_grid};
import package::shaders::lib::input::plot::{FragmentInput};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // Antialiased polyline coverage in [0,1] via smoothstep SDF.
    let line_coverage = draw_line(uv, 0.003);

    // Bright green line composited over a fully transparent background.
    let line_color = vec4<f32>(0.1, 0.8, 0.1, line_coverage);

    return line_color;
}
