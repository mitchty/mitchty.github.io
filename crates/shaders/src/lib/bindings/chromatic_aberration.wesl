import package::lib::types::chromatic_aberration::{ChromaticAberrationSettings};

@if(WGPU_TEST)
@group(0) @binding(0) var<uniform> settings: ChromaticAberrationSettings;
@else
@group(0) @binding(0) var screen_texture: texture_2d<f32>;

// TODO: this is a bit jank
@if(!WGPU_TEST)
@group(0) @binding(1) var texture_sampler: sampler;

@if(!WGPU_TEST && UI_MATERIAL)
@group(1) @binding(0) var<uniform> settings: ChromaticAberrationSettings;

@if(!WGPU_TEST && !UI_MATERIAL)
@group(0) @binding(2) var<uniform> settings: ChromaticAberrationSettings;
