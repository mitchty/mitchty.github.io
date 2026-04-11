import package::flan::shaders::lib::bindings::plot::{params, points, point_data};
import package::flan::shaders::lib::helpers::plot::{get_point, point_count, draw_line};
import package::flan::shaders::lib::input::plot::{FragmentInput};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // Background colour: deep dark blue
    let bg = vec3<f32>(0.05, 0.05, 0.12);

    // Grid lines
    let grid_x = fract(uv.x * 10.0);
    let grid_y = fract(uv.y * 10.0);
    let grid = f32(grid_x < 0.02 || grid_y < 0.02) * 0.15;

    // Plot the polyline using the SDF helper.
    let line = draw_line(uv);

    let col = bg + vec3<f32>(grid)
            + vec3<f32>(0.1, 0.8, 0.1) * line;

    return vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
