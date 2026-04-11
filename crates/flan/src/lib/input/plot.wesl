@if(WGPU_TEST)
struct FragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@if(!WGPU_TEST && UI_MATERIAL)
struct FragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
    @location(1)       color:    vec4<f32>,
}

@if(!WGPU_TEST && !UI_MATERIAL)
struct FragmentInput {
    @builtin(position) position:       vec4<f32>,
    @location(0)       world_position: vec4<f32>,
    @location(1)       world_normal:   vec3<f32>,
    @location(2)       uv:             vec2<f32>,
}
