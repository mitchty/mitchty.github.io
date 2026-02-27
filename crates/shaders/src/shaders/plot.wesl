import package::lib::bindings::plot::{params};
import package::lib::input::plot::{FragmentInput};

// Analytic distance from `p` to the animated sin wave:
//   y = sin(x * 10.0 + time) * 0.4 + 0.5
// params.time drives a horizontal phase shift so the wave travels over time.
fn dist_to_sin_wave(p: vec2<f32>) -> f32 {
    let wave_y = sin(p.x * 10.0 + params.time) * 0.4 + 0.5;
    return abs(p.y - wave_y);
}

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let d = dist_to_sin_wave(in.uv);

    if d < 0.008 {
        return vec4<f32>(0.2, 0.8, 1.0, 1.0);
    }
    // Transparent background
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
