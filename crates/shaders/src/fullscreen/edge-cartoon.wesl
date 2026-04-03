// Edge-Cartoon effect.
// Ported from https://www.shadertoy.com/view/Mc2Szt
//
// Technique: Sobel or color-difference edge detection + luminance
// quantization for a cel-shaded / inked cartoon look.
import package::shaders::lib::bindings::fullscreen_effect::{settings};
import package::shaders::lib::input::fullscreen_effect::{FragmentInput};

@if(!WGPU_TEST)
import package::shaders::lib::bindings::fullscreen_effect::{screen_texture, texture_sampler};

const LINE_SIZE      : f32  = 1.2;
const EDGE_THRESHOLD : f32  = 0.01;

@if(!WGPU_TEST)
fn rgb2lum(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
}

// Simple color-difference edge detection
// Returns 1.0 if no edge, 0.0 if edge.
@if(!WGPU_TEST)
fn edgeDetection(uv: vec2<f32>, color: vec3<f32>, texel_size: vec2<f32>, edge_size: f32) -> f32 {
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            if x == 0 && y == 0 { continue; }
            let offset         = vec2<f32>(f32(x), f32(y)) * edge_size;
            let neighbor_color = textureSample(screen_texture, texture_sampler, uv + offset * texel_size).rgb;
            let similarity     = dot(
                normalize(color + vec3<f32>(0.001)),
                normalize(neighbor_color + vec3<f32>(0.001))
            );
            if similarity <= 0.99 { return 0.0; }
        }
    }
    return 1.0;
}

// Sobel luminance gradient. Need to figure out my wesl lib layout
// TODO: Mitch refactor things to be less derp, a lot of these functions are useful in general.
@if(!WGPU_TEST)
fn calculateSobel(uv: vec2<f32>, texel_size: vec2<f32>, edge_size: f32) -> vec2<f32> {
    var dirs: array<vec2<f32>, 9>;
    dirs[0] = vec2<f32>(-1.0,  1.0);
    dirs[1] = vec2<f32>( 0.0,  1.0);
    dirs[2] = vec2<f32>( 1.0,  1.0);
    dirs[3] = vec2<f32>(-1.0,  0.0);
    dirs[4] = vec2<f32>( 0.0,  0.0);
    dirs[5] = vec2<f32>( 1.0,  0.0);
    dirs[6] = vec2<f32>(-1.0, -1.0);
    dirs[7] = vec2<f32>( 0.0, -1.0);
    dirs[8] = vec2<f32>( 1.0, -1.0);

    var sobelX: array<f32, 9>;
    sobelX[0] = -1.0; sobelX[1] = 0.0; sobelX[2] = 1.0;
    sobelX[3] = -2.0; sobelX[4] = 0.0; sobelX[5] = 2.0;
    sobelX[6] = -1.0; sobelX[7] = 0.0; sobelX[8] = 1.0;

    var sobelY: array<f32, 9>;
    sobelY[0] =  1.0; sobelY[1] =  2.0; sobelY[2] =  1.0;
    sobelY[3] =  0.0; sobelY[4] =  0.0; sobelY[5] =  0.0;
    sobelY[6] = -1.0; sobelY[7] = -2.0; sobelY[8] = -1.0;

    let center_lum = rgb2lum(textureSample(screen_texture, texture_sampler, uv).rgb);
    var gradX = 0.0;
    var gradY = 0.0;

    for (var i = 0; i < 9; i++) {
        let sample_uv   = uv + dirs[i] * edge_size * texel_size;
        let lum_diff    = abs(center_lum - rgb2lum(textureSample(screen_texture, texture_sampler, sample_uv).rgb));
        gradX += lum_diff * sobelX[i];
        gradY += lum_diff * sobelY[i];
    }

    return vec2<f32>(gradX, gradY);
}

// Quantise color to discrete cartoon shading levels.
@if(!WGPU_TEST)
fn cartoonShading(color: vec3<f32>, intensity: f32) -> vec3<f32> {
    let lum           = rgb2lum(color);
    let levels        = mix(8.0, 3.0, intensity);
    let quantized_lum = floor(lum * levels) / levels;
    let scale         = (quantized_lum + 0.1) / (lum + 0.1);
    return color * scale;
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
    let resolution  = vec2<f32>(textureDimensions(screen_texture));
    let texel_size  = 1.0 / resolution;
    let uv          = in.position.xy / resolution;

    let original_color = textureSample(screen_texture, texture_sampler, uv).rgb;

    // Sobel edge detection with a bit more computation
    let line_size      = mix(0.5, LINE_SIZE, settings.intensity);
    let sobel_result   = calculateSobel(uv, texel_size, line_size);
    let edge_factor    = 1.0 - step(EDGE_THRESHOLD, length(sobel_result));

    // Cartoon color quantization
    let cartoon_color  = cartoonShading(original_color, settings.intensity);

    // Blend edges aka black to give the effect of a hand drawn thing at definable edges
    let effect_color   = mix(vec3<f32>(0.0), cartoon_color, edge_factor);

    let final_color    = mix(original_color, effect_color, settings.intensity);
    return vec4<f32>(final_color, 1.0);
}
