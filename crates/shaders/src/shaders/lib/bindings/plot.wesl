import package::lib::types::plot::{PlotUniform};
@if(WEBGL)
import package::lib::types::plot::{PlotPointsUniform};

@if(WGPU_TEST)
@group(0) @binding(0) var<uniform> params: PlotUniform;

@if(!WGPU_TEST && UI_MATERIAL)
@group(1) @binding(0) var<uniform> params: PlotUniform;

@if(!WGPU_TEST && !UI_MATERIAL)
@group(2) @binding(0) var<uniform> params: PlotUniform;

@if(WGPU_TEST && WEBGL)
@group(0) @binding(1) var<uniform> point_data: PlotPointsUniform;

@if(WGPU_TEST && !WEBGL)
@group(0) @binding(1) var<storage, read> points: array<vec2<f32>>;

@if(!WGPU_TEST && UI_MATERIAL && WEBGL)
@group(1) @binding(1) var<uniform> point_data: PlotPointsUniform;

@if(!WGPU_TEST && UI_MATERIAL && !WEBGL)
@group(1) @binding(1) var<storage, read> points: array<vec2<f32>>;

@if(!WGPU_TEST && !UI_MATERIAL && WEBGL)
@group(2) @binding(1) var<uniform> point_data: PlotPointsUniform;

@if(!WGPU_TEST && !UI_MATERIAL && !WEBGL)
@group(2) @binding(1) var<storage, read> points: array<vec2<f32>>;
