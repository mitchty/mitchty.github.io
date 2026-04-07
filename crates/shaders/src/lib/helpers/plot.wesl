import package::shaders::lib::bindings::plot::{params, points, point_data};

// Return the index-th sample from the point data buffer.
// In WEBGL mode, point_data is a uniform array<vec4<f32>, 512> where .xy are the coordinates.
// In non-WEBGL mode, points is a storage array<vec2<f32>>.
@if(WEBGL)
fn get_point(index: u32) -> vec2<f32> {
    return point_data.data[index].xy;
}
@else
fn get_point(index: u32) -> vec2<f32> {
    return points[index];
}

fn point_count() -> u32 {
    return params.count;
}

// Line sdf for a line segment from p to q evaluated at point d. Returns the
// unsigned distance from d to the nearest point on the segment.
fn line_sdf(d: vec2<f32>, p: vec2<f32>, q: vec2<f32>) -> f32 {
    let pq = q - p;
    let t = clamp(dot(d - p, pq) / dot(pq, pq), 0.0, 1.0);
    return length(d - p - pq * t);
}

// Draw a line through all provided data points.
// Returns a coverage value in [0, 1] - 1 = fully on the line, 0 = fully off.
// Antialiased half-width is read from params.line_width so each PlotUiMaterial
// instance can specify an appropriate width for its pixel size without needing
// entry-point shaders to import the bindings directly.
fn draw_line(uv: vec2<f32>) -> f32 {
    let n = point_count();
    var min_dist: f32 = 1e9;

    for (var i: u32 = 0u; i + 1u < n; i += 1u) {
        let p0 = get_point(i);
        let p1 = get_point(i + 1u);
        min_dist = min(min_dist, line_sdf(uv, p0, p1));
    }

    return 1.0 - smoothstep(0.0, params.line_width, min_dist);
}

// Draw a regular grid with `divisions` cells along each axis.
// Returns a coverage value in [0, 1] - 1 = fully on a grid line, 0 = off.
// line_width controls the antialiased half-width of each grid line in UV space.
fn draw_grid(uv: vec2<f32>, divisions: f32, line_width: f32) -> f32 {
    let cell = fract(uv * divisions);
    // Distance to the nearest grid edge in each axis, normalised to [0, 0.5].
    let d = min(cell, 1.0 - cell) / divisions;
    let dist = min(d.x, d.y);
    return 1.0 - smoothstep(0.0, line_width, dist);
}
