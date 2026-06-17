//! Plot shader sparkline, scatter-line ish renderer. Its just a dam line nothing special.
//!
//! Two initial variants:
//!
//! * `plot_default`  `@group(1)`, points in a `RuntimeArray<Vec2f>` storage buffer.
//! * `plot_texture`  `@group(1)`, points packed into a `PlotPointsUniform`
//!   array of `vec4<f32>` (std140). Texture path.
//!
//! Both modules expose a `WGSL_MODULE` constant and `wgsl_source()` via
//! `wgsl-rs`. Register the one you want in `ShadersPlugin` and pair it
//! with the matching Bevy `UiMaterial` type in `lib.rs`.
// Next refactors going to make one single plugin out of all this and clean up
// the room to not have Materials all over and loosely match the Bevy feature
// set for material definition/setup too.
use wgsl_rs::wgsl;

/// Helpers shared by both plot shaders.
#[wgsl]
pub mod plot_helpers {
    use wgsl_rs::std::*;

    /// Distance from `d` to the nearest point on segment `p -> q`.
    pub fn line_sdf_inner(d: Vec2f, p: Vec2f, q: Vec2f) -> f32 {
        let pq = q - p;
        let t = clamp(dot(d - p, pq) / dot(pq, pq), 0.0, 1.0);
        length(d - p - pq * t)
    }

    /// Antialiased grid-line coverage for a UV coordinate.
    ///
    /// Returns 1.0 on a grid-line edge, 0.0 in the cell interior.
    pub fn grid_coverage_inner(uv: Vec2f, divisions: f32, line_width: f32) -> f32 {
        let cell = fract(uv * divisions);
        let d = min(cell, vec2f(1.0, 1.0) - cell) / divisions;
        let dist = min(d.x(), d.y());
        1.0 - smoothstep(0.0, line_width, dist)
    }

    /// Accumulate one segment's contribution into the running minimum distance.
    pub fn update_min_dist(current: f32, uv: Vec2f, p0: Vec2f, p1: Vec2f) -> f32 {
        min(current, line_sdf_inner(uv, p0, p1))
    }

    /// Convert a minimum SDF distance to an antialiased coverage value.
    pub fn coverage_from_dist(min_dist: f32, line_width: f32) -> f32 {
        1.0 - smoothstep(0.0, line_width, min_dist)
    }

    /// Extract a `Vec2f` point from one element of the texture-variant packed uniform.
    ///
    /// Points are stored as `vec4<f32>` with `(x, y, 0, 0)` for std140 alignment.
    pub fn unpack_point(v: Vec4f) -> Vec2f {
        vec2f(v.x(), v.y())
    }

    /// wgsl alias for [`line_sdf_inner`].
    pub fn line_sdf(d: Vec2f, p: Vec2f, q: Vec2f) -> f32 {
        line_sdf_inner(d, p, q)
    }

    /// wgsl alias for [`grid_coverage_inner`].
    pub fn draw_grid(uv: Vec2f, divisions: f32, line_width: f32) -> f32 {
        grid_coverage_inner(uv, divisions, line_width)
    }
}

/// Plot `UiMaterial` shader - native / WebGPU path.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  PlotUniform
/// binding(1)  storage  points  RuntimeArray<Vec2f>
/// ```
#[wgsl]
pub mod plot_default {
    use super::plot_helpers::*;
    use wgsl_rs::std::*;

    #[derive(Wgsl)]
    pub struct PlotUniform {
        pub min: Vec2f,
        pub max: Vec2f,
        pub zoom: Vec2f,
        pub offset: Vec2f,
        pub count: u32,
        pub _pad: f32,
        pub line_width: f32,
    }

    uniform!(group(1), binding(0), PARAMS: PlotUniform);
    storage!(group(1), binding(1), POINTS: RuntimeArray<Vec2f>);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
        #[location(1)]
        pub color: Vec4f,
    }

    pub fn get_point(index: u32) -> Vec2f {
        get!(POINTS)[index as usize]
    }

    pub fn draw_line(uv: Vec2f) -> f32 {
        let n = get!(PARAMS).count;
        let mut dist: f32 = 1e9;
        let mut i: u32 = 0;
        while i + 1 < n {
            dist = update_min_dist(dist, uv, get_point(i), get_point(i + 1));
            i += 1;
        }
        coverage_from_dist(dist, get!(PARAMS).line_width)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        vec4f(0.0, 0.0, 0.0, draw_line(input.uv))
    }
}

/// Plot `UiMaterial` shader - WebGL2-compatible path.
///
/// Points are stored in a `PlotPointsUniform` array of `vec4<f32>` (std140).
/// `unpack_point` extracts `.xy` from each element.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  PlotUniform
/// binding(1)  uniform  PlotPointsUniform  { data: array<vec4<f32>, MAX_PLOT_POINTS> }
/// ```
#[wgsl]
pub mod plot_texture {
    use super::plot_helpers::*;
    use wgsl_rs::std::*;

    #[derive(Wgsl)]
    pub struct PlotUniform {
        pub min: Vec2f,
        pub max: Vec2f,
        pub zoom: Vec2f,
        pub offset: Vec2f,
        pub count: u32,
        pub _pad: f32,
        pub line_width: f32,
    }

    /// std140 uniform array of plot points.
    ///
    /// Size must match `MAX_PLOT_POINTS` in `build.rs` / `constants.rs` (512).
    /// Written as a literal because wgsl-rs cannot resolve `crate::` paths
    /// inside a `#[wgsl]` module - it would mangle them into invalid WGSL
    /// identifiers.
    #[derive(Wgsl)]
    pub struct PlotPointsUniform {
        pub data: [Vec4f; 512],
    }

    uniform!(group(1), binding(0), PARAMS: PlotUniform);
    uniform!(group(1), binding(1), POINT_DATA: PlotPointsUniform);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
        #[location(1)]
        pub color: Vec4f,
    }

    pub fn get_point(index: u32) -> Vec2f {
        unpack_point(get!(POINT_DATA).data[index as usize])
    }

    pub fn draw_line(uv: Vec2f) -> f32 {
        let n = get!(PARAMS).count;
        let mut dist: f32 = 1e9;
        let mut i: u32 = 0;
        while i + 1 < n {
            dist = update_min_dist(dist, uv, get_point(i), get_point(i + 1));
            i += 1;
        }
        coverage_from_dist(dist, get!(PARAMS).line_width)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        vec4f(0.0, 0.0, 0.0, draw_line(input.uv))
    }
}

#[cfg(test)]
mod tests {
    use super::plot_helpers::*;
    use wgsl_rs::std::*;

    #[test]
    fn line_sdf_zero_at_midpoint() {
        let d = line_sdf_inner(vec2f(0.5, 0.0), vec2f(0.0, 0.0), vec2f(1.0, 0.0));
        assert!(d < 1e-6, "expected ~0, got {d}");
    }

    #[test]
    fn line_sdf_perpendicular_distance() {
        let d = line_sdf_inner(vec2f(0.5, 0.5), vec2f(0.0, 0.0), vec2f(1.0, 0.0));
        assert!((d - 0.5).abs() < 1e-6, "expected ~0.5, got {d}");
    }

    #[test]
    fn line_sdf_clamps_before_start() {
        let d = line_sdf_inner(vec2f(-1.0, 0.0), vec2f(0.0, 0.0), vec2f(1.0, 0.0));
        assert!((d - 1.0).abs() < 1e-6, "expected ~1.0, got {d}");
    }

    #[test]
    fn line_sdf_clamps_beyond_end() {
        let d = line_sdf_inner(vec2f(2.0, 0.0), vec2f(0.0, 0.0), vec2f(1.0, 0.0));
        assert!((d - 1.0).abs() < 1e-6, "expected ~1.0, got {d}");
    }

    #[test]
    fn update_min_dist_decreases_on_closer_segment() {
        let uv = vec2f(0.5, 0.5);
        let far = update_min_dist(1e9, uv, vec2f(0.0, 0.0), vec2f(1.0, 0.0));
        assert!((far - 0.5).abs() < 1e-6, "expected ~0.5, got {far}");
    }

    #[test]
    fn update_min_dist_keeps_existing_when_farther() {
        let uv = vec2f(0.5, 0.5);
        let result = update_min_dist(0.1, uv, vec2f(10.0, 10.0), vec2f(11.0, 10.0));
        assert!((result - 0.1).abs() < 1e-6, "expected ~0.1, got {result}");
    }

    #[test]
    fn coverage_full_at_zero_distance() {
        let c = coverage_from_dist(0.0, 0.01);
        assert!((c - 1.0).abs() < 1e-6, "expected 1.0, got {c}");
    }

    #[test]
    fn coverage_zero_beyond_line_width() {
        let c = coverage_from_dist(1.0, 0.01);
        assert!(c < 1e-6, "expected ~0, got {c}");
    }

    #[test]
    fn coverage_half_at_half_line_width() {
        let c = coverage_from_dist(0.05, 0.1);
        assert!((c - 0.5).abs() < 1e-4, "expected ~0.5, got {c}");
    }

    #[test]
    fn unpack_point_extracts_xy() {
        let v = vec4f(0.3, 0.7, 0.0, 0.0);
        let p = unpack_point(v);
        assert!((p.x() - 0.3).abs() < 1e-7, "x wrong: {}", p.x());
        assert!((p.y() - 0.7).abs() < 1e-7, "y wrong: {}", p.y());
    }

    #[test]
    fn unpack_point_ignores_zw() {
        let p = unpack_point(vec4f(0.1, 0.2, 99.0, -99.0));
        assert!((p.x() - 0.1).abs() < 1e-7);
        assert!((p.y() - 0.2).abs() < 1e-7);
    }

    #[test]
    fn grid_coverage_near_zero_at_cell_center() {
        let c = grid_coverage_inner(vec2f(0.05, 0.05), 10.0, 0.01);
        assert!(c < 0.01, "expected near-zero at cell center, got {c}");
    }

    #[test]
    fn grid_coverage_full_at_boundary() {
        let c = grid_coverage_inner(vec2f(0.0, 0.0), 10.0, 0.01);
        assert!(c > 0.99, "expected ~1.0 at grid boundary, got {c}");
    }

    #[test]
    fn max_plot_points_is_positive() {
        const { assert!(crate::MAX_PLOT_POINTS > 0) };
    }
}
