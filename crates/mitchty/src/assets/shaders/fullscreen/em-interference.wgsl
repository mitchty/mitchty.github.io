// EM Interference shader...ish sorta Creates a glitchy, interference-like
// effect with scanlines and color separation kinda like chromatic abberration
// with picking where the abberation happens abusing time

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture : texture_2d<f32>;
@group(0) @binding(1) var texture_sampler : sampler;

struct FullScreenEffect {
  intensity : f32,
              time : f32,
#ifdef SIXTEEN_BYTE_ALIGNMENT
                     // WebGL2 structs must be 16 byte aligned.
                     _webgl2_padding : vec2<f32>
#endif
}

@group(0) @binding(2) var<uniform> settings : FullScreenEffect;

fn rng2(seed : vec2<f32>) -> f32 {
  return fract(
      sin(dot(seed * floor(settings.time * 12.0), vec2<f32>(127.1, 311.7))) *
      43758.5453123);
}

fn rng(seed : f32) -> f32 { return rng2(vec2<f32>(seed, 1.0)); }

@fragment fn fragment(in : FullscreenVertexOutput) -> @location(0) vec4<f32> {
  // Pass through unchanged if intensity is 0
  // if settings
  //   .intensity <= 0.0 {
  //     return textureSample(screen_texture, texture_sampler, in.uv);
  //   }

  let resolution = vec2<f32>(textureDimensions(screen_texture));
  let uv = in.position.xy / resolution;

  let blockS = floor(uv * vec2<f32>(24.0, 9.0));
  let blockL = floor(uv * vec2<f32>(8.0, 4.0));

  let r = rng2(uv);
  let noise = (vec3<f32>(r, 1.0 - r, r / 2.0 + 0.5) * 1.0 - 2.0) * 0.08;

  let lineNoise = pow(rng2(blockS), 8.0) * pow(rng2(blockL), 3.0) -
                  pow(rng(7.2341), 17.0) * 2.0;

  let col1 = textureSample(screen_texture, texture_sampler, uv);
  let col2 = textureSample(
      screen_texture, texture_sampler,
      uv + vec2<f32>(lineNoise * 0.05 * rng(5.0) * settings.intensity, 0.0));
  let col3 = textureSample(
      screen_texture, texture_sampler,
      uv - vec2<f32>(lineNoise * 0.05 * rng(31.0) * settings.intensity, 0.0));

  return vec4<f32>(
      vec3<f32>(col1.x, col2.y, col3.z) + noise * settings.intensity, 1.0);
}
