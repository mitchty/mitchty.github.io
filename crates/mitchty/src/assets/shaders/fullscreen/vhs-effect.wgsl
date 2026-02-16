// VHS Effect shader - ported from GLSL Shadertoy
// Creates a retro VHS tape effect with vertical bars, noise, and chromatic
// aberration

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture : texture_2d<f32>;
@group(0) @binding(1) var texture_sampler : sampler;

struct FullScreenEffect {
  intensity : f32,
              time : f32,
#ifdef SIXTEEN_BYTE_ALIGNMENT
                     _webgl2_padding : vec2<f32>
#endif
}

@group(0) @binding(2) var<uniform> settings : FullScreenEffect;

// Constants
const RANGE : f32 = 0.05;
const NOISE_QUALITY : f32 = 250.0;
const NOISE_INTENSITY : f32 = 0.0088;
const OFFSET_INTENSITY : f32 = 0.02;
const COLOR_OFFSET_INTENSITY : f32 = 1.3;

// Random number generator
fn rand(co : vec2<f32>) -> f32 {
  return fract(sin(dot(co, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Vertical bar distortion
fn vertical_bar(pos : f32, uv_y : f32, offset : f32) -> f32 {
  let edge0 = pos - RANGE;
  let edge1 = pos + RANGE;

  var x = smoothstep(edge0, pos, uv_y) * offset;
  x -= smoothstep(pos, edge1, uv_y) * offset;
  return x;
}

@fragment fn fragment(in : FullscreenVertexOutput) -> @location(0) vec4<f32> {
  // Pass through unchanged if intensity is 0
  // if settings
  //   .intensity <= 0.0 {
  //     return textureSample(screen_texture, texture_sampler, in.uv);
  //   }

  let resolution = vec2<f32>(textureDimensions(screen_texture));
  var uv = in.uv;

  // Apply vertical bar distortions
  var i : f32 = 0.0;
  loop {
    if i
      >= 0.71 { break; }

    let d = (settings.time * i) % 1.7;
    var o = sin(1.0 - tan(settings.time * 0.24 * i));
    o *= OFFSET_INTENSITY * settings.intensity;
    uv.x += vertical_bar(d, uv.y, o);

    i += 0.1313;
  }

  // Add horizontal noise
  var uv_y = uv.y;
  uv_y *= NOISE_QUALITY;
  uv_y = f32(i32(uv_y)) * (1.0 / NOISE_QUALITY);
  let noise = rand(vec2<f32>(settings.time * 0.00001, uv_y));
  uv.x += noise * NOISE_INTENSITY * settings.intensity;

  // Chromatic aberration offsets
  let offset_r = vec2<f32>(0.006 * sin(settings.time), 0.0) *
                 COLOR_OFFSET_INTENSITY * settings.intensity;
  let offset_g = vec2<f32>(0.0073 * cos(settings.time * 0.97), 0.0) *
                 COLOR_OFFSET_INTENSITY * settings.intensity;

  // Sample each color channel with offsets
  let r = textureSample(screen_texture, texture_sampler, uv + offset_r).r;
  let g = textureSample(screen_texture, texture_sampler, uv + offset_g).g;
  let b = textureSample(screen_texture, texture_sampler, uv).b;

  return vec4<f32>(r, g, b, 1.0);
}
