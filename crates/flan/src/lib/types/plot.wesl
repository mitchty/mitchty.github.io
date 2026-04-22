struct PlotUniform {
    min:        vec2<f32>,
    max:        vec2<f32>,
    zoom:       vec2<f32>,
    offset:     vec2<f32>,
    // Number of valid points to render.
    count:      u32,
    // Elapsed time in seconds used to animate the shader.
    time:            f32,
    // @size(8) pads time to 8 bytes so webgl works transparently
    @size(8) line_width: f32,
}

// webgl uniform padded to 16 bytes, vec4 is intentional cause std140 pads
// array<vec2> to 16 bytes anyway, so non webgl should be a storage buffer
@if(WEBGL)
struct PlotPointsUniform {
    // Must match MAX_PLOT_POINTS in flan/src/lib.rs dumdum!
    data: array<vec4<f32>, 512>,
}
