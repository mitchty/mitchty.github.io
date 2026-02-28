import package::lib::bindings::plot::{params, points, point_data};
import package::lib::helpers::plot::{min_dist_to_polyline};
import package::lib::input::plot::{FragmentInput};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let d = min_dist_to_polyline(in.uv);

    if d < 0.008 {
        return vec4<f32>(0.2, 0.8, 1.0, 1.0);
    }
    // Transparent background
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
