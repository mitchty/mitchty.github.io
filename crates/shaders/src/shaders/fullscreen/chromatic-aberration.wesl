// Note, this was all yeeted/inspired from the bevy example code
//
// I mostly just ported all this to a wesl setup.

// Splits the RGB channels of the input screen texture by a small UV offset
// proportional to settings.intensity, producing the classic colour-fringing
// lens aberration look.
//
// In WGPU_TEST mode no screen texture is available, so the shader just
// returns a solid grey whose brightness is derived from settings.intensity
// — enough to confirm the shader compiles and can be exercised by the
// headless test renderer without needing texture sampling support.
import package::lib::bindings::chromatic_aberration::{settings};
import package::lib::input::chromatic_aberration::{FragmentInput};

@if(!WGPU_TEST)
import package::lib::bindings::chromatic_aberration::{screen_texture, texture_sampler};

// WGPU_TEST variant: no screen texture available in the headless test
// harness so output a grey value derived from intensity to confirm the
// compilation path works and the snapshot machinery can produce a reference.
@if(WGPU_TEST)
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let v = clamp(settings.intensity / 10.0, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}

// Real variant: sample the screen texture with per-channel RGB offsets.
@if(!WGPU_TEST)
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
