// intensity: controls how far the RGB channels are offset from each other (unused for now)
// time: reserved for future animated variants (ignored for now)
@if(WEBGL)
struct ChromaticAberrationSettings {
    intensity:       f32,
    time:            f32,
    _webgl2_padding: vec2<f32>,
}

@else
struct ChromaticAberrationSettings {
    intensity: f32,
    time:      f32,
}
