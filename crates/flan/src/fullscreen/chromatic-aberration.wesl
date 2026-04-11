// Chromatic aberration effect shader, yeeted from bevy examples but it
// basically splits RGB channels by a UV offset scaled to intensity.
import package::flan::shaders::lib::bindings::fullscreen_effect::{settings};
import package::flan::shaders::lib::input::fullscreen_effect::{FragmentInput};

@if(!WGPU_TEST)
import package::flan::shaders::lib::bindings::fullscreen_effect::{screen_texture, texture_sampler};

@if(WGPU_TEST)
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let v = clamp(settings.intensity / 10.0, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}
@else
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let offset_strength = settings.intensity * 0.005;
    let r = textureSample(screen_texture, texture_sampler,
                          in.uv + vec2<f32>(offset_strength, -offset_strength)).r;
    let g = textureSample(screen_texture, texture_sampler,
                          in.uv + vec2<f32>(-offset_strength, 0.0)).g;
    let b = textureSample(screen_texture, texture_sampler,
                          in.uv + vec2<f32>(0.0, offset_strength)).b;
    return vec4<f32>(r, g, b, 1.0);
}
