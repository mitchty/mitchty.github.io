// VHS tape effect, vertical bar distortion, scanline noise, chromatic offsets.
// Ported from the original WGSL/GLSL Shadertoy-inspired implementation but now
// is single pass vs multiple with glsl.
import package::shaders::lib::bindings::fullscreen_effect::{settings};
import package::shaders::lib::input::fullscreen_effect::{FragmentInput};

@if(!WGPU_TEST)
import package::shaders::lib::bindings::fullscreen_effect::{screen_texture, texture_sampler};

const RANGE: f32 = 0.05;
const NOISE_QUALITY: f32 = 250.0;
const NOISE_INTENSITY: f32 = 0.0088;
const OFFSET_INTENSITY: f32 = 0.02;
const COLOR_OFFSET_INTENSITY: f32 = 1.3;

fn rand(co: vec2<f32>) -> f32 {
    return fract(sin(dot(co, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn vertical_bar(pos: f32, uv_y: f32, offset: f32) -> f32 {
    let edge0 = pos - RANGE;
    let edge1 = pos + RANGE;
    var x = smoothstep(edge0, pos, uv_y) * offset;
    x -= smoothstep(pos, edge1, uv_y) * offset;
    return x;
}

@if(WGPU_TEST)
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let v = clamp(settings.intensity / 10.0, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}
@else
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    var uv = in.uv;

    var i: f32 = 0.0;
    loop {
        if i >= 0.71 { break; }
        let d = (settings.time * i) % 1.7;
        var o = sin(1.0 - tan(settings.time * 0.24 * i));
        o *= OFFSET_INTENSITY * settings.intensity;
        uv.x += vertical_bar(d, uv.y, o);
        i += 0.1313;
    }

    var uv_y = uv.y;
    uv_y *= NOISE_QUALITY;
    uv_y = f32(i32(uv_y)) * (1.0 / NOISE_QUALITY);
    let noise = rand(vec2<f32>(settings.time * 0.00001, uv_y));
    uv.x += noise * NOISE_INTENSITY * settings.intensity;

    let offset_r = vec2<f32>(0.006 * sin(settings.time), 0.0) * COLOR_OFFSET_INTENSITY * settings.intensity;
    let offset_g = vec2<f32>(0.0073 * cos(settings.time * 0.97), 0.0) * COLOR_OFFSET_INTENSITY * settings.intensity;

    let r = textureSample(screen_texture, texture_sampler, uv + offset_r).r;
    let g = textureSample(screen_texture, texture_sampler, uv + offset_g).g;
    let b = textureSample(screen_texture, texture_sampler, uv).b;

    return vec4<f32>(r, g, b, 1.0);
}
