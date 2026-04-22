// Shared settings struct for all fullscreen post-process effects.
// intensity: effect strength 0.0 = passthrough, higher = stronger ish
// time: animated effects use this; static effects ignore it
struct FullscreenEffectSettings {
    intensity:     f32,
    // @size(12) pads time to 12 bytes so webgl works transparently
    @size(12) time: f32,
}
