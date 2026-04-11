// Shared settings struct for all fullscreen post-process effects.
// intensity: effect strength 0.0 = passthrough, higher = stronger ish
// time: animated effects use this; static effects ignore it
@if(WEBGL)
struct FullscreenEffectSettings {
    intensity:       f32,
    time:            f32,
    _webgl2_padding: vec2<f32>,
}
@else
struct FullscreenEffectSettings {
    intensity: f32,
    time:      f32,
}
