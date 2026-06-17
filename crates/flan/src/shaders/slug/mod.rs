//! Slug text shader.
use wgsl_rs::wgsl;

pub mod text;
pub mod text3d;

/// Math helpers and layout constants.
#[wgsl]
pub mod slug_helpers {
    use wgsl_rs::std::*;

    /// Determine which roots of a quadratic bezier y-crossing are valid.
    ///
    /// Returns bitmask: bit 0 = root t1 eligible, bit 8 = root t2 eligible.
    /// Uses the 0x2E74 lookup table encoding all 8 sign combinations of
    /// (y1, y2, y3). Direct port of `SlugCalcRootCode` from the Slug HLSL.
    ///
    /// TODO: Rust ! is bitwise not for integers but wgsl uses ~. wgsl-rs
    /// emits `!` verbatim so the `& !N` masks are hacked with a shift to make
    /// an equivalent. I need to open a pr to fix this at some point cause this
    /// was a nightmare to fix.
    pub fn slug_calc_root_code_inner(y1: f32, y2: f32, y3: f32) -> u32 {
        let i1 = bitcast_u32(y1) >> 31u32;
        let i2 = bitcast_u32(y2) >> 30u32;
        let i3 = bitcast_u32(y3) >> 29u32;

        let mut shift = (i2 & 2u32) | i1;
        shift |= i3 & 4u32;

        (0x2E74u32 >> shift) & 0x0101u32
    }

    /// Solve x values where the quadratic Bezier crosses y = 0 horizontal sweep.
    ///
    /// `p12 = (p0.x, p0.y, p1.x, p1.y)`, `p3 = (p2.x, p2.y)`.
    /// Returns `(x_at_t1, x_at_t2)` in em-space.
    /// Near-degenerate (a.y ~= 0) falls back to a linear solve.
    pub fn slug_solve_horiz_poly_inner(p12: Vec4f, p3: Vec2f) -> Vec2f {
        let p0xy = vec2f(p12.x(), p12.y());
        let p1xy = vec2f(p12.z(), p12.w());
        let a = p0xy - p1xy * 2.0 + p3;
        let b = p0xy - p1xy;
        let ra = 1.0 / a.y();
        let rb = 0.5 / b.y();

        let d = sqrt(max(b.y() * b.y() - a.y() * p12.y(), 0.0));
        let mut t1 = (b.y() - d) * ra;
        let mut t2 = (b.y() + d) * ra;

        if abs(a.y()) < (1.0 / 65536.0) {
            t1 = p12.y() * rb;
            t2 = p12.y() * rb;
        }

        vec2f(
            (a.x() * t1 - b.x() * 2.0) * t1 + p12.x(),
            (a.x() * t2 - b.x() * 2.0) * t2 + p12.x(),
        )
    }

    /// Solve y values where the quadratic Bezier crosses x = 0 vertical sweep.
    ///
    /// Symmetric to `slug_solve_horiz_poly_inner` with x/y roles swapped.
    pub fn slug_solve_vert_poly_inner(p12: Vec4f, p3: Vec2f) -> Vec2f {
        let p0xy = vec2f(p12.x(), p12.y());
        let p1xy = vec2f(p12.z(), p12.w());
        let a = p0xy - p1xy * 2.0 + p3;
        let b = p0xy - p1xy;
        let ra = 1.0 / a.x();
        let rb = 0.5 / b.x();

        let d = sqrt(max(b.x() * b.x() - a.x() * p12.x(), 0.0));
        let mut t1 = (b.x() - d) * ra;
        let mut t2 = (b.x() + d) * ra;

        if abs(a.x()) < (1.0 / 65536.0) {
            t1 = p12.x() * rb;
            t2 = p12.x() * rb;
        }

        vec2f(
            (a.y() * t1 - b.y() * 2.0) * t1 + p12.y(),
            (a.y() * t2 - b.y() * 2.0) * t2 + p12.y(),
        )
    }

    /// Combine horizontal and vertical winding accumulations into a scalar
    /// coverage value in `[0, 1]` using the nonzero fill rule.
    ///
    /// Weighted average falls back to `min(|xcov|, |ycov|)` when one direction
    /// has no close crossings (weight near zero).
    pub fn slug_calc_coverage_inner(xcov: f32, ycov: f32, xwgt: f32, ywgt: f32) -> f32 {
        let coverage = max(
            abs(xcov * xwgt + ycov * ywgt) / max(xwgt + ywgt, 1.0 / 65536.0),
            min(abs(xcov), abs(ycov)),
        );
        clamp(coverage, 0.0, 1.0)
    }

    // Bits [1:0] = Vertical, bits [3:2] = Horizontal.
    //   Vertical:   Center=0  Top=1  Bottom=2
    //   Horizontal: Center=0  Left=1 Right=2  Fill=3

    pub const SLUG_LAYOUT_VCENTER: u32 = 0u32;
    pub const SLUG_LAYOUT_VTOP: u32 = 1u32;
    pub const SLUG_LAYOUT_VBOTTOM: u32 = 2u32;

    pub const SLUG_LAYOUT_HCENTER: u32 = 0u32;
    pub const SLUG_LAYOUT_HLEFT: u32 = 1u32;
    pub const SLUG_LAYOUT_HRIGHT: u32 = 2u32;
    pub const SLUG_LAYOUT_HFILL: u32 = 3u32;

    pub const SLUG_LAYOUT_CENTER: u32 = 0x00u32;
    pub const SLUG_LAYOUT_LEFT: u32 = 0x04u32;
    pub const SLUG_LAYOUT_RIGHT: u32 = 0x08u32;
    pub const SLUG_LAYOUT_FILL: u32 = 0x0Cu32;
    pub const SLUG_LAYOUT_TOP_LEFT: u32 = 0x05u32;
    pub const SLUG_LAYOUT_TOP_CENTER: u32 = 0x01u32;
    pub const SLUG_LAYOUT_TOP_RIGHT: u32 = 0x09u32;
    pub const SLUG_LAYOUT_BOTTOM_LEFT: u32 = 0x06u32;
    pub const SLUG_LAYOUT_BOTTOM_CENTER: u32 = 0x02u32;
    pub const SLUG_LAYOUT_BOTTOM_RIGHT: u32 = 0x0Au32;

    /// wgsl alias for [`slug_calc_root_code_inner`].
    pub fn slug_calc_root_code(y1: f32, y2: f32, y3: f32) -> u32 {
        slug_calc_root_code_inner(y1, y2, y3)
    }

    /// wgsl alias for [`slug_solve_horiz_poly_inner`].
    pub fn slug_solve_horiz_poly(p12: Vec4f, p3: Vec2f) -> Vec2f {
        slug_solve_horiz_poly_inner(p12, p3)
    }

    /// wgsl alias for [`slug_solve_vert_poly_inner`].
    pub fn slug_solve_vert_poly(p12: Vec4f, p3: Vec2f) -> Vec2f {
        slug_solve_vert_poly_inner(p12, p3)
    }

    /// wgsl  alias for [`slug_calc_coverage_inner`].
    pub fn slug_calc_coverage(xcov: f32, ycov: f32, xwgt: f32, ywgt: f32) -> f32 {
        slug_calc_coverage_inner(xcov, ycov, xwgt, ywgt)
    }
}

#[cfg(test)]
mod tests {
    use super::slug_helpers::*;
    use wgsl_rs::std::*;

    #[test]
    fn coverage_full_inside() {
        let c = slug_calc_coverage_inner(1.0, 1.0, 1.0, 1.0);
        assert!((c - 1.0).abs() < 1e-6, "expected 1.0, got {c}");
    }

    #[test]
    fn coverage_zero_outside() {
        let c = slug_calc_coverage_inner(0.0, 0.0, 1.0, 1.0);
        assert!(c < 1e-6, "expected ~0, got {c}");
    }

    #[test]
    fn coverage_clamped_to_one() {
        let c = slug_calc_coverage_inner(5.0, 5.0, 1.0, 1.0);
        assert!((c - 1.0).abs() < 1e-6, "expected 1.0 (clamped), got {c}");
    }

    #[test]
    fn coverage_zero_weight_falls_back_to_min() {
        let c = slug_calc_coverage_inner(1.0, 0.0, 0.0, 0.0);
        assert!(c < 1e-5, "expected ~0 from min fallback, got {c}");
    }

    #[test]
    fn root_code_all_positive_is_zero() {
        let code = slug_calc_root_code_inner(1.0, 1.0, 1.0);
        assert_eq!(code, 0, "all-positive should give code 0, got {code}");
    }

    #[test]
    fn root_code_all_negative_is_zero() {
        let code = slug_calc_root_code_inner(-1.0, -1.0, -1.0);
        assert_eq!(code, 0, "all-negative should give code 0, got {code}");
    }

    #[test]
    fn root_code_straddle_gives_nonzero() {
        let code = slug_calc_root_code_inner(-1.0, 0.0, 1.0);
        assert_ne!(code, 0, "straddling curve should give nonzero code");
    }

    #[test]
    fn horiz_poly_returns_finite_values() {
        let p12 = vec4f(0.0, -1.0, 0.5, 0.0);
        let p3 = vec2f(1.0, 1.0);
        let r = slug_solve_horiz_poly_inner(p12, p3);
        assert!(r.x().is_finite(), "t1 x result must be finite: {}", r.x());
        assert!(r.y().is_finite(), "t2 x result must be finite: {}", r.y());
    }

    #[test]
    fn horiz_poly_degenerate_no_nan() {
        // Near-linear a.y = 1 - 2*0.5 + 0 = 0 -> degenerate branch
        let p12 = vec4f(0.0, 1.0, 0.5, 0.5);
        let p3 = vec2f(1.0, 0.0);
        let r = slug_solve_horiz_poly_inner(p12, p3);
        assert!(!r.x().is_nan(), "degenerate branch must not produce NaN");
        assert!(!r.y().is_nan(), "degenerate branch must not produce NaN");
    }

    #[test]
    fn vert_poly_returns_finite_values() {
        let p12 = vec4f(-1.0, 0.0, 0.0, 0.5);
        let p3 = vec2f(1.0, 1.0);
        let r = slug_solve_vert_poly_inner(p12, p3);
        assert!(r.x().is_finite(), "t1 y result must be finite: {}", r.x());
        assert!(r.y().is_finite(), "t2 y result must be finite: {}", r.y());
    }

    #[test]
    fn layout_constants_are_distinct() {
        assert_ne!(SLUG_LAYOUT_HLEFT, SLUG_LAYOUT_HRIGHT);
        assert_ne!(SLUG_LAYOUT_HLEFT, SLUG_LAYOUT_HFILL);
        assert_ne!(SLUG_LAYOUT_HCENTER, SLUG_LAYOUT_HLEFT);
        const { assert!(SLUG_LAYOUT_VTOP < 4) };
        const { assert!(SLUG_LAYOUT_VBOTTOM < 4) };
    }
}
