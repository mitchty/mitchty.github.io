import package::lib::bindings::plot::{params, points, point_data};
import package::lib::helpers::plot::{draw_line_sdf};
import package::lib::input::plot::{FragmentInput};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    // TODO: Need to brain on how I pass this data into the shader
    // Big q right now is how I plan to handle having multiple plots like
    // traderview etc....
    let half_width = 0.002;
    let aa         = 0.001;
    let line_color = vec4<f32>(0.2, 0.8, 1.0, 1.0);
    let bg_color   = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return draw_line_sdf(in.uv, half_width, aa, line_color, bg_color);
}
