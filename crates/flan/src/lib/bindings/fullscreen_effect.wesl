// Bindings for fullscreen post-process effects.
// Binding layouts for not wgpu stuff:
//   @group(0) @binding(0) = screen_texture
//   @group(0) @binding(1) = texture_sampler
//   @group(0) @binding(2) = settings  (non-UI_MATERIAL)
//   @group(1) @binding(0) = settings  (UI_MATERIAL only)
import package::flan::shaders::lib::types::fullscreen_effect::{FullscreenEffectSettings};

@if(WGPU_TEST)
@group(0) @binding(0) var<uniform> settings: FullscreenEffectSettings;
@else
@group(0) @binding(0) var screen_texture: texture_2d<f32>;

@if(!WGPU_TEST)
@group(0) @binding(1) var texture_sampler: sampler;

@if(!WGPU_TEST && UI_MATERIAL)
@group(1) @binding(0) var<uniform> settings: FullscreenEffectSettings;

@if(!WGPU_TEST && !UI_MATERIAL)
@group(0) @binding(2) var<uniform> settings: FullscreenEffectSettings;
