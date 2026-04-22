// intensity: controls how far the RGB channels are offset from each other (unused for now)
// time: reserved for future animated variants (ignored for now)
struct ChromaticAberrationSettings {
    intensity:     f32,
    // @size(12) pads time to 12 bytes so webgl works transparently
    @size(12) time: f32,
}
