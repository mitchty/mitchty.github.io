import package::lib::bindings::plot::{params, points, point_data};
import package::lib::helpers::plot::{get_point, point_count, min_dist_to_polyline};
import package::lib::input::plot::{FragmentInput};

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let d = min_dist_to_polyline(in.uv);

    if d < 0.01 {
        // opaque cyan line to draw
        return vec4<f32>(0.2, 0.8, 1.0, 1.0);
    }
    // transparent background, more for ui materials
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
