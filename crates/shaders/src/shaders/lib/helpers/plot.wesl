import package::lib::bindings::plot::{params, points, point_data};

// Return the i-th point in data-space.
@if(WEBGL)
fn get_point(i: u32) -> vec2<f32> {
    return point_data.data[i].xy;
}

@if(!WEBGL)
fn get_point(i: u32) -> vec2<f32> {
    return points[i];
}

// Return the number of valid points.
// On WEBGL the storage buffer API (arrayLength) is unavailable, so use
// params.count which we get from ye olde rust setup.
@if(WEBGL)
fn point_count() -> u32 {
    return params.count;
}

@if(!WEBGL)
fn point_count() -> u32 {
    return arrayLength(&points);
}

// Minimum distance from `p` to the segment [a, b] in UV space.
fn dist_to_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h  = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Minimum distance from `p` to any segment of the polyline defined by the
// current point buffer.
fn min_dist_to_polyline(p: vec2<f32>) -> f32 {
    let n = point_count();
    var d = 1000.0;
    for (var i = 0u; i < n - 1u; i++) {
        d = min(d, dist_to_segment(p, get_point(i), get_point(i + 1u)));
    }
    return d;
}
