import package::lib::bindings::plot::{params, points, point_data};
import package::lib::helpers::plot::{get_point, point_count, draw_line_sdf};
import package::lib::input::plot::{FragmentInput};

// TODO: This might be better yeeted into a lib function.

// Draw a grid line at every `step` interval in uv space.
// Returns 1.0 when `v` is within `half_width` of a grid line, else 0.0.
fn grid_line(v: f32, step: f32, half_width: f32) -> f32 {
    let d = abs(v % step - 0.5 * step);
    return select(0.0, 1.0, d < half_width);
}

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // uv gradient background: red channel = U, green channel = V.
    var color = vec4<f32>(uv.x * 0.6, uv.y * 0.6, 0.15, 1.0);

    // Faint grid every 0.1 uv units, 0.5 px wide in uv space.
    let gx = grid_line(uv.x, 0.1, 0.003);
    let gy = grid_line(uv.y, 0.1, 0.003);
    let g  = max(gx, gy);
    color = mix(color, vec4<f32>(1.0, 1.0, 1.0, 1.0), g * 0.25);

    // Thicker lines every 0.5 uv units.
    let gx2 = grid_line(uv.x, 0.5, 0.005);
    let gy2 = grid_line(uv.y, 0.5, 0.005);
    let g2  = max(gx2, gy2);
    color = mix(color, vec4<f32>(1.0, 1.0, 1.0, 1.0), g2 * 0.5);

    // White SDF line with mixed in color based off background
    let half_width = 0.012;
    let aa         = 0.001;
    let line_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    return draw_line_sdf(uv, half_width, aa, line_color, color);
}
