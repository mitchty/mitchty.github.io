// Computes a surface normal from image luminance gradients and applies diffuse
// and specular directional lighting and a vignette to give the impression of
// thick oil-paint brushstrokes.
import package::shaders::lib::bindings::fullscreen_effect::{settings};
import package::shaders::lib::input::fullscreen_effect::{FragmentInput};

@if(!WGPU_TEST)
import package::shaders::lib::bindings::fullscreen_effect::{screen_texture, texture_sampler};

@if(!WGPU_TEST)
fn get_val(uv: vec2<f32>, res: vec2<f32>) -> f32 {
    // The post-process screen texture is a single-mip render target, so
    // textureSampleLevel at mip > 0 returns black and kills the gradient.
    // Instead, sample at mip 0 and use a slightly wider tap radius scaled by
    // resolution to approximate the smoothing the original got from LOD.
    let col = textureSampleLevel(screen_texture, texture_sampler, uv, 0.0);
    return length(col.xyz);
}

@if(!WGPU_TEST)
fn get_grad(uv: vec2<f32>, delta: f32, res: vec2<f32>) -> vec2<f32> {
    let d = vec2<f32>(delta, 0.0);
    return vec2<f32>(
        get_val(uv + d.xy, res) - get_val(uv - d.xy, res),
        get_val(uv + d.yx, res) - get_val(uv - d.yx, res)
    ) / delta;
}

// TODO: I really gotta figure out a way to render and test against something in
// these wgpu tests.
@if(WGPU_TEST)
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let v = clamp(settings.intensity / 10.0, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}

@else
@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    let res = vec2<f32>(textureDimensions(screen_texture));
    let uv  = in.position.xy / res;

    // Surface normal from luminance gradient Use a wider tap distance of 3px to
    // compensate for sampling mip 0 instead of a blurred mip level; this keeps
    // the gradient from being dominated by single-pixel noise from the
    // fragment.
    let grad = get_grad(uv, 3.0 / res.y, res);
    // Smaller z -> more pronounced bumps. Scale with intensity so the effect
    // is controllable: at intensity 0 the surface is very flat z=200,
    // at intensity 1 it's embossed at z=30.
    let z_flat = mix(200.0, 30.0, clamp(settings.intensity, 0.0, 1.0));
    var n = normalize(vec3<f32>(grad, z_flat));

    let light = normalize(vec3<f32>(-1.0, 1.0, 1.4));

    let diff = clamp(dot(n, light), 0.0, 1.0);

    // Scale specular highlight by intensity so it grows with the effect.
    let paint_spec = 0.15 * clamp(settings.intensity, 0.0, 1.0);
    let spec_raw = clamp(dot(reflect(light, n), vec3<f32>(0.0, 0.0, -1.0)), 0.0, 1.0);
    let spec     = pow(spec_raw, 12.0) * paint_spec;

    let sh_raw = clamp(dot(reflect(light * vec3<f32>(-1.0, -1.0, 1.0), n), vec3<f32>(0.0, 0.0, -1.0)), 0.0, 1.0);
    let sh     = pow(sh_raw, 4.0) * 0.1;

    let base = textureSample(screen_texture, texture_sampler, uv);
    let highlight = vec4<f32>(0.85, 1.0, 1.15, 1.0);
    var color = base * mix(diff, 1.0, 0.9) + spec * highlight + sh * highlight;
    color.w = 1.0;

    let vign_strength = 1.5;
    let scc = (in.position.xy - 0.5 * res) / res.x;
    var vign = 1.1 - vign_strength * dot(scc, scc);
    vign *= 1.0 - 0.7 * vign_strength * exp(-sin(uv.x * 3.14159265) * 40.0);
    vign *= 1.0 - 0.7 * vign_strength * exp(-sin(uv.y * 3.14159265) * 20.0);
    color = vec4<f32>(color.xyz * vign, 1.0);

    return color;
}
