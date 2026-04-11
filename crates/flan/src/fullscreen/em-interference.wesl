// EM Interference effect, glitchy scanline noise with color separation.
import package::flan::shaders::lib::bindings::fullscreen_effect::{settings};
import package::flan::shaders::lib::input::fullscreen_effect::{FragmentInput};

@if(!WGPU_TEST)
import package::flan::shaders::lib::bindings::fullscreen_effect::{screen_texture, texture_sampler};

fn rng2(seed: vec2<f32>) -> f32 {
    return fract(
        sin(dot(seed * floor(settings.time * 12.0), vec2<f32>(127.1, 311.7))) *
        43758.5453123
    );
}

fn rng(seed: f32) -> f32 {
    return rng2(vec2<f32>(seed, 1.0));
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
    let resolution = vec2<f32>(textureDimensions(screen_texture));
    let uv = in.position.xy / resolution;

    let blockS = floor(uv * vec2<f32>(24.0, 9.0));
    let blockL = floor(uv * vec2<f32>(8.0, 4.0));

    let r = rng2(uv);
    let noise = (vec3<f32>(r, 1.0 - r, r / 2.0 + 0.5) * 1.0 - 2.0) * 0.08;

    let lineNoise = pow(rng2(blockS), 8.0) * pow(rng2(blockL), 3.0) -
                    pow(rng(7.2341), 17.0) * 2.0;

    let col1 = textureSample(screen_texture, texture_sampler, uv);
    let col2 = textureSample(screen_texture, texture_sampler,
        uv + vec2<f32>(lineNoise * 0.05 * rng(5.0) * settings.intensity, 0.0));
    let col3 = textureSample(screen_texture, texture_sampler,
        uv - vec2<f32>(lineNoise * 0.05 * rng(31.0) * settings.intensity, 0.0));

    return vec4<f32>(vec3<f32>(col1.x, col2.y, col3.z) + noise * settings.intensity, 1.0);
}
