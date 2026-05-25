// Slug algorithm core maths.
//
// Direct WGSL port of SlugPixelShader.hlsl by Eric Lengyel (MIT License 2017).
// Ref: https://jcgt.org/published/0006/02/02/

// Most of the comments are reflections of the hlsl comments. I read the paper
// and porting helped understand the approach but I'm still not 100% on all this
// stuff.

// CalcRootCode determines which roots of a quadratic Bezier y-crossing are valid.
//
// Given three y/x control point values of a Bezier that is shifted so the
// sample point is at the origin, returns a bitmask indicating eligibility:
//   bit 0 set -> root t1 is eligible
//   bit 8 set -> root t2 is eligible
//
// The 0x2E74 lookup table encodes all 8 sign combinations of (y1,y2,y3) as a tuple.
fn slug_calc_root_code(y1: f32, y2: f32, y3: f32) -> u32 {
    let i1 = bitcast<u32>(y1) >> 31u;
    let i2 = bitcast<u32>(y2) >> 30u;
    let i3 = bitcast<u32>(y3) >> 29u;

    var shift = (i2 & 2u) | (i1 & ~2u);
    shift     = (i3 & 4u) | (shift & ~4u);

    return (0x2E74u >> shift) & 0x0101u;
}

// SolveHorizPoly solves x values where the Bezier crosses y = 0.
//
// p12 = vec4(p0.x, p0.y, p1.x, p1.y)  first two control points
// p3  = vec2(p2.x, p2.y)              third control point
//
// Returns vec2(x_at_t1, x_at_t2) in em-space.
// The caller should subtract the sample position before calling so the curve
// is relative to the current fragment in the font parser for curves.
fn slug_solve_horiz_poly(p12: vec4<f32>, p3: vec2<f32>) -> vec2<f32> {
    // Quadratic polynomial coefficients: a t^2 - 2b t + c
    // a = p0 - 2*p1 + p2,  b = p0 - p1,  c = p0
    let a = p12.xy - p12.zw * 2.0 + p3;
    let b = p12.xy - p12.zw;
    let ra = 1.0 / a.y;
    let rb = 0.5 / b.y;

    let d = sqrt(max(b.y * b.y - a.y * p12.y, 0.0));
    var t1 = (b.y - d) * ra;
    var t2 = (b.y + d) * ra;

    // Near-linear degenerate: solve for -2b*t + c = 0
    if abs(a.y) < (1.0 / 65536.0) {
        t1 = p12.y * rb;
        t2 = p12.y * rb;
    }

    // Evaluate x component of C(t) = (a x t - 2 b x)*t + p0.x
    return vec2<f32>(
        (a.x * t1 - b.x * 2.0) * t1 + p12.x,
        (a.x * t2 - b.x * 2.0) * t2 + p12.x,
    );
}

// SolveVertPoly solve for y values where the Bezier crosses x = 0.
// Symmetric to ^^^ but operates on x/y swapped instead.
fn slug_solve_vert_poly(p12: vec4<f32>, p3: vec2<f32>) -> vec2<f32> {
    let a = p12.xy - p12.zw * 2.0 + p3;
    let b = p12.xy - p12.zw;
    let ra = 1.0 / a.x;
    let rb = 0.5 / b.x;

    let d = sqrt(max(b.x * b.x - a.x * p12.x, 0.0));
    var t1 = (b.x - d) * ra;
    var t2 = (b.x + d) * ra;

    if abs(a.x) < (1.0 / 65536.0) {
        t1 = p12.x * rb;
        t2 = p12.x * rb;
    }

    return vec2<f32>(
        (a.y * t1 - b.y * 2.0) * t1 + p12.y,
        (a.y * t2 - b.y * 2.0) * t2 + p12.y,
    );
}

// CalcCoverage combine horizontal and vertical winding number accumulations.
//
// xcov/ycov = accumulated winding numbers from h/v ray sweeps.
// xwgt/ywgt = closeness weights aka how near each ray's crossings were to center.
//
// Uses nonzero fill rule and no antialiasing smoothstep, the smoothstep pass is up
// to the caller so it can be chosen to be on/off or not. Returns coverage within [0, 1].
fn slug_calc_coverage(xcov: f32, ycov: f32, xwgt: f32, ywgt: f32) -> f32 {
    // Weighted average of the two winding numbers, fallbacks to min when one
    // direction has no close crossings.
    let coverage = max(
        abs(xcov * xwgt + ycov * ywgt) / max(xwgt + ywgt, 1.0 / 65536.0),
        min(abs(xcov), abs(ycov)),
    );
    // Nonzero fill rule means clamp to [0, 1].
    return clamp(coverage, 0.0, 1.0);
}
