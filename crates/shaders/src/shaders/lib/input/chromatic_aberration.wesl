// Post processing chromatic abberation shader

struct FragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}
