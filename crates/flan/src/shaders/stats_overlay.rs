//! Stats overlay shader sparkline + slug text combined widget.
//!
//! Two GPU variants:
//!
//! * `stats_overlay_default`  - `@group(1)`, data in storage buffers.
//!
//! * `stats_overlay_texture`  - `@group(1)`, data packed into 2-D `rgba32`
//!   textures read with `textureLoad`. Use on WebGL2.
//!
//! Both modules expose a `WGSL_MODULE` constant and `wgsl_source()` via
//! `wgsl-rs`. Register the one you want in `ShadersPlugin` and pair it
//! with the matching Bevy `UiMaterial` type in `lib.rs`.
use wgsl_rs::wgsl;

#[wgsl]
pub mod stats_overlay_helpers {
    use wgsl_rs::std::*;

    pub const PLOT_FRAC: f32 = 0.43478261; // 100 / 230
    pub const COLOR_MODE_COLOR: u32 = 0u32;
    pub const COLOR_MODE_INVERT: u32 = 1u32;

    /// `true` when `uv.x` falls in the plot (left-hand) sub-region.
    pub fn in_plot_region(uv: Vec2f) -> bool {
        uv.x() < PLOT_FRAC
    }

    /// Remap the full node UV into plot-region local `[0,1]²`.
    pub fn to_plot_uv(uv: Vec2f) -> Vec2f {
        vec2f(uv.x() / PLOT_FRAC, uv.y())
    }

    /// Remap the full node UV into text-region local `[0,1]²`.
    pub fn to_text_uv(uv: Vec2f) -> Vec2f {
        let text_frac = 1.0 - PLOT_FRAC;
        vec2f((uv.x() - PLOT_FRAC) / text_frac, uv.y())
    }

    /// Normalize a raw FPS value to `[0, 1]` given the observed range.
    pub fn normalize_fps(fps: f32, min_fps: f32, max_fps: f32) -> f32 {
        let range = max_fps - min_fps;
        if range < 1e-6 {
            return 0.5;
        }
        clamp((fps - min_fps) / range, 0.0, 1.0)
    }

    /// X coordinate in `[0, 1]` for point index `i` out of `count`.
    pub fn plot_point_x(i: u32, count: u32) -> f32 {
        if count <= 1u32 {
            return 0.0;
        }
        f32(i) / f32(count - 1u32)
    }

    pub fn stats_overlay_shade(
        coverage: f32,
        alpha_discard: f32,
        color_mode: u32,
        text_color: Vec4f,
        background: Vec4f,
    ) -> Vec4f {
        if color_mode == COLOR_MODE_INVERT {
            if coverage < alpha_discard {
                discard!();
            }
            return vec4f(coverage, coverage, coverage, 1.0);
        }
        if coverage < alpha_discard {
            return background;
        }
        text_color * coverage
    }
}

#[wgsl]
pub mod stats_overlay_types {
    use wgsl_rs::std::*;

    /// Overlay params uniform - 64 bytes.
    ///
    /// ```text
    /// offset  0: node_size         vec2<f32>  (8)
    /// offset  8: min_fps           f32        (4)
    /// offset 12: max_fps           f32        (4)
    /// offset 16: line_width        f32        (4)
    /// offset 20: layout_flags      u32        (4)
    /// offset 24: alpha_discard     f32        (4)
    /// offset 28: color_mode        u32        (4)
    /// offset 32: text_color        vec4<f32>  (16)
    /// offset 48: background_color  vec4<f32>  (16)
    /// total = 64 bytes
    /// ```
    #[derive(Wgsl)]
    pub struct StatsOverlayUniform {
        pub node_size: Vec2f,
        pub min_fps: f32,
        pub max_fps: f32,
        pub line_width: f32,
        pub layout_flags: u32,
        pub alpha_discard: f32,
        pub color_mode: u32,
        pub text_color: Vec4f,
        pub background_color: Vec4f,
    }

    /// 80-byte GPU glyph record (5 x `vec4<u32>`).
    #[derive(Copy, Clone, Wgsl)]
    pub struct GlyphInfo {
        pub data: Vec4u,
        pub vband: [u32; 8],
        pub hband: [u32; 8],
    }

    /// Per-glyph layout: 48 bytes = 3 x `vec4<f32>`.
    #[derive(Copy, Clone, Wgsl)]
    pub struct SlugGlyphLayout {
        pub screen_rect: Vec4f,
        pub em_rect: Vec4f,
        pub glyph_index: u32,
        pub _pad: [u32; 3],
    }

    /// Per-run descriptor: 16 bytes = 1 x `vec4<f32>`.
    #[derive(Copy, Clone, Wgsl)]
    pub struct SlugRunDesc {
        pub natural_advance: f32,
        pub natural_height: f32,
        pub glyph_offset: u32,
        pub glyph_count: u32,
    }
}

/// Stats overlay UiMaterial shader.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  StatsOverlayUniform     (64bytes)
/// binding(1)  storage  fps_points              RuntimeArray<f32>
/// binding(2)  storage  slug_curves             RuntimeArray<[Vec2f; 3]>
/// binding(3)  storage  slug_curve_indices      RuntimeArray<u32>
/// binding(4)  storage  slug_glyphs             RuntimeArray<GlyphInfo>
/// binding(5)  storage  slug_runs               RuntimeArray<SlugRunDesc>
/// binding(6)  storage  slug_glyph_layout       RuntimeArray<SlugGlyphLayout>
/// ```
#[wgsl]
#[allow(unused_assignments)]
pub mod stats_overlay_default {
    use super::super::plot::plot_helpers::*;
    use super::super::slug::slug_helpers::*;
    use super::stats_overlay_helpers::*;
    use super::stats_overlay_types::*;
    use wgsl_rs::std::*;

    uniform!(group(1), binding(0), OVERLAY_PARAMS: StatsOverlayUniform);
    storage!(group(1), binding(1), FPS_POINTS:         RuntimeArray<f32>);
    storage!(group(1), binding(2), SLUG_CURVES:        RuntimeArray<[Vec2f; 3]>);
    storage!(group(1), binding(3), SLUG_CURVE_INDICES: RuntimeArray<u32>);
    storage!(group(1), binding(4), SLUG_GLYPHS:        RuntimeArray<GlyphInfo>);
    storage!(group(1), binding(5), SLUG_RUNS:          RuntimeArray<SlugRunDesc>);
    storage!(group(1), binding(6), SLUG_GLYPH_LAYOUT:  RuntimeArray<SlugGlyphLayout>);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    pub fn get_fps_point(i: u32) -> f32 {
        get!(FPS_POINTS)[i as usize]
    }

    pub fn get_curve(idx: u32) -> [Vec2f; 3] {
        get!(SLUG_CURVES)[idx as usize]
    }

    pub fn get_curve_index(idx: u32) -> u32 {
        get!(SLUG_CURVE_INDICES)[idx as usize]
    }

    pub fn get_glyph(idx: u32) -> GlyphInfo {
        get!(SLUG_GLYPHS)[idx as usize]
    }

    pub fn get_run(idx: u32) -> SlugRunDesc {
        get!(SLUG_RUNS)[idx as usize]
    }

    pub fn get_glyph_layout(idx: u32) -> SlugGlyphLayout {
        get!(SLUG_GLYPH_LAYOUT)[idx as usize]
    }

    pub fn slug_fetch_band_ranges(em_pos: Vec2f, glyph_idx: u32) -> Vec4u {
        const BAND_COUNT: f32 = 8.0;
        let g = get_glyph(glyph_idx);
        let d_xy = g.data.x();
        let d_wh = g.data.y();
        let bbox_min = vec2f(f32(i32(d_xy << 16u32) >> 16u32), f32(i32(d_xy) >> 16u32));
        let bbox_max = vec2f(f32(i32(d_wh << 16u32) >> 16u32), f32(i32(d_wh) >> 16u32));
        let size = max(bbox_max - bbox_min, vec2f(0.0001, 0.0001));
        let band_f = (em_pos - bbox_min) * (BAND_COUNT / size);
        let bx = u32(clamp(i32(band_f.x()), 0i32, i32(BAND_COUNT) - 1i32));
        let by = u32(clamp(i32(band_f.y()), 0i32, i32(BAND_COUNT) - 1i32));
        let h_packed = g.hband[by as usize];
        let v_packed = g.vband[bx as usize];
        vec4u(
            h_packed & 0x00FFFFFFu32,
            h_packed >> 24u32,
            v_packed & 0x00FFFFFFu32,
            v_packed >> 24u32,
        )
    }

    pub fn slug_render_glyph(em_pos: Vec2f, band_ranges: Vec4u, pixels_per_em: Vec2f) -> f32 {
        let mut xcov: f32 = 0.0;
        let mut xwgt: f32 = 0.0;
        let h_base = band_ranges.x();
        let h_count = band_ranges.y();
        let mut ci: u32 = 0;
        while ci < h_count {
            let curve_idx = get_curve_index(h_base + ci);
            let curve = get_curve(curve_idx);
            let p0 = curve[0] - em_pos;
            let p1 = curve[1] - em_pos;
            let p2 = curve[2] - em_pos;
            if max(max(p0.x(), p1.x()), p2.x()) * pixels_per_em.x() < -0.5 {
                break;
            }
            let code = slug_calc_root_code(p0.y(), p1.y(), p2.y());
            if code != 0u32 {
                let p12 = vec4f(p0.x(), p0.y(), p1.x(), p1.y());
                let r = slug_solve_horiz_poly(p12, p2) * pixels_per_em.x();
                if (code & 1u32) != 0u32 {
                    xcov += clamp(r.x() + 0.5, 0.0, 1.0);
                    xwgt = max(xwgt, clamp(1.0 - abs(r.x()) * 2.0, 0.0, 1.0));
                }
                if code > 1u32 {
                    xcov -= clamp(r.y() + 0.5, 0.0, 1.0);
                    xwgt = max(xwgt, clamp(1.0 - abs(r.y()) * 2.0, 0.0, 1.0));
                }
            }
            ci += 1;
        }
        let mut ycov: f32 = 0.0;
        let mut ywgt: f32 = 0.0;
        let v_base = band_ranges.z();
        let v_count = band_ranges.w();
        let mut ci2: u32 = 0;
        while ci2 < v_count {
            let curve_idx = get_curve_index(v_base + ci2);
            let curve = get_curve(curve_idx);
            let p0 = curve[0] - em_pos;
            let p1 = curve[1] - em_pos;
            let p2 = curve[2] - em_pos;
            if max(max(p0.y(), p1.y()), p2.y()) * pixels_per_em.y() < -0.5 {
                break;
            }
            let code = slug_calc_root_code(p0.x(), p1.x(), p2.x());
            if code != 0u32 {
                let p12 = vec4f(p0.x(), p0.y(), p1.x(), p1.y());
                let r = slug_solve_vert_poly(p12, p2) * pixels_per_em.y();
                if (code & 1u32) != 0u32 {
                    ycov -= clamp(r.x() + 0.5, 0.0, 1.0);
                    ywgt = max(ywgt, clamp(1.0 - abs(r.x()) * 2.0, 0.0, 1.0));
                }
                if code > 1u32 {
                    ycov += clamp(r.y() + 0.5, 0.0, 1.0);
                    ywgt = max(ywgt, clamp(1.0 - abs(r.y()) * 2.0, 0.0, 1.0));
                }
            }
            ci2 += 1;
        }
        slug_calc_coverage(xcov, ycov, xwgt, ywgt)
    }

    pub fn slugtext(px: Vec2f, rect: Vec4f, layout_flags: u32, run_idx: u32) -> f32 {
        if px.x() < rect.x() || px.x() > rect.z() || px.y() < rect.y() || px.y() > rect.w() {
            return 0.0;
        }
        let run = get_run(run_idx);
        if run.glyph_count == 0u32 {
            return 0.0;
        }
        let region = vec2f(rect.z() - rect.x(), rect.w() - rect.y());
        let v_bits = layout_flags & 0x3u32;
        let h_bits = (layout_flags >> 2u32) & 0x3u32;
        let adv = max(run.natural_advance, 0.001);
        let hgt = max(run.natural_height, 0.001);
        let scale_w = region.x() / adv;
        let scale_h = region.y() / hgt;
        let mut em_scale_x: f32 = 0.0;
        let mut em_scale_y: f32 = 0.0;
        if h_bits == 3u32 {
            em_scale_x = scale_w;
            em_scale_y = scale_h;
        } else {
            let s = min(scale_w, scale_h);
            em_scale_x = s;
            em_scale_y = s;
        }
        let scaled_advance = adv * em_scale_x;
        let mut h_off: f32 = 0.0;
        if h_bits == 1u32 || h_bits == 3u32 {
            h_off = 0.0;
        } else if h_bits == 2u32 {
            h_off = region.x() - scaled_advance;
        } else {
            h_off = (region.x() - scaled_advance) * 0.5;
        }
        let scaled_height = hgt * em_scale_y;
        let mut v_off: f32 = 0.0;
        if v_bits == 1u32 || h_bits == 3u32 {
            v_off = 0.0;
        } else if v_bits == 2u32 {
            v_off = region.y() - scaled_height;
        } else {
            v_off = (region.y() - scaled_height) * 0.5;
        }
        let origin = vec2f(rect.x() + h_off, rect.y() + v_off);
        let glyph_local = (px - origin) / vec2f(em_scale_x, em_scale_y);
        let g0 = get_glyph_layout(run.glyph_offset);
        let screen_size = g0.screen_rect.zw() - g0.screen_rect.xy();
        let em_size = abs(g0.em_rect.zw() - g0.em_rect.xy());
        let base_ppe = screen_size / max(em_size, vec2f(0.001, 0.001));
        let pixels_per_em = base_ppe * vec2f(em_scale_x, em_scale_y);
        let mut coverage: f32 = 0.0;
        let end = run.glyph_offset + run.glyph_count;
        let mut i = run.glyph_offset;
        while i < end {
            let g = get_glyph_layout(i);
            if glyph_local.x() < g.screen_rect.x() || glyph_local.x() > g.screen_rect.z() {
                i += 1;
                continue;
            }
            if glyph_local.y() < g.screen_rect.y() || glyph_local.y() > g.screen_rect.w() {
                i += 1;
                continue;
            }
            let t = (glyph_local - g.screen_rect.xy())
                / max(g.screen_rect.zw() - g.screen_rect.xy(), vec2f(0.001, 0.001));
            let em = mix(g.em_rect.xy(), g.em_rect.zw(), t);
            let band_ranges = slug_fetch_band_ranges(em, g.glyph_index);
            coverage = max(coverage, slug_render_glyph(em, band_ranges, pixels_per_em));
            i += 1;
        }
        coverage
    }

    pub fn draw_sparkline(plot_uv: Vec2f, min_fps: f32, max_fps: f32, line_width: f32) -> f32 {
        let count: u32 = 256u32;
        let mut dist: f32 = 1e9;
        let pad = line_width + 0.005;
        let mut i: u32 = 0u32;
        while i + 1u32 < count {
            let raw0 = get_fps_point(i);
            let raw1 = get_fps_point(i + 1u32);
            if raw0 > 0.0 && raw1 > 0.0 {
                let y0 = clamp(normalize_fps(raw0, min_fps, max_fps), pad, 1.0 - pad);
                let y1 = clamp(normalize_fps(raw1, min_fps, max_fps), pad, 1.0 - pad);
                let p0 = vec2f(plot_point_x(i, count), 1.0 - y0);
                let p1 = vec2f(plot_point_x(i + 1u32, count), 1.0 - y1);
                dist = update_min_dist(dist, plot_uv, p0, p1);
            }
            i += 1;
        }
        coverage_from_dist(dist, line_width)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let uv = input.uv;
        let params = get!(OVERLAY_PARAMS);
        let background = params.background_color;
        let text_color = params.text_color;
        let layout_flags = params.layout_flags;
        let alpha_discard = params.alpha_discard;
        let color_mode = params.color_mode;
        let node_size = params.node_size;
        let min_fps = params.min_fps;
        let max_fps = params.max_fps;
        let line_width = params.line_width;

        if in_plot_region(uv) {
            let plot_uv = to_plot_uv(uv);
            let coverage = draw_sparkline(plot_uv, min_fps, max_fps, line_width);
            stats_overlay_shade(coverage, alpha_discard, color_mode, text_color, background)
        } else {
            let text_uv = to_text_uv(uv);
            let text_frac = 1.0 - PLOT_FRAC;
            let text_w = node_size.x() * text_frac;
            let text_h = node_size.y();
            let px = text_uv * vec2f(text_w, text_h);
            let rect = vec4f(0.0, 0.0, text_w, text_h);
            let coverage = slugtext(px, rect, layout_flags, 0u32);
            stats_overlay_shade(coverage, alpha_discard, color_mode, text_color, background)
        }
    }
}

/// Stats overlay UiMaterial shader - WebGL-compatible (texture) path.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  StatsOverlayUniform   (64bytes)
/// binding(1)  texture  fps_points_tex         rgba32float  64x1  (256 f32 packed 4-per-texel)
/// binding(2)  texture  slug_curves_tex        rgba32float  2 texels/curve
/// binding(3)  texture  slug_curve_indices_tex rgba32uint   4 u32/texel
/// binding(4)  texture  slug_glyphs_tex        rgba32uint   5 texels/GlyphInfo
/// binding(5)  texture  slug_runs_tex          rgba32float  1 texel/SlugRunDesc
/// binding(6)  texture  slug_glyph_layout_tex  rgba32float  3 texels/SlugGlyphLayout
/// ```
/// All textures are 2048 px wide and as tall as needed.
#[wgsl]
#[allow(unused_assignments)]
pub mod stats_overlay_texture {
    use super::super::plot::plot_helpers::*;
    use super::super::slug::slug_helpers::*;
    use super::stats_overlay_helpers::*;
    use super::stats_overlay_types::*;
    use wgsl_rs::std::*;

    uniform!(group(1), binding(0), OVERLAY_PARAMS:          StatsOverlayUniform);
    texture!(group(1), binding(1), FPS_POINTS_TEX:          Texture2D<f32>);
    texture!(group(1), binding(2), SLUG_CURVES_TEX:         Texture2D<f32>);
    texture!(group(1), binding(3), SLUG_CURVE_INDICES_TEX:  Texture2D<u32>);
    texture!(group(1), binding(4), SLUG_GLYPHS_TEX:         Texture2D<u32>);
    texture!(group(1), binding(5), SLUG_RUNS_TEX:           Texture2D<f32>);
    texture!(group(1), binding(6), SLUG_GLYPH_LAYOUT_TEX:   Texture2D<f32>);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    // All textures are 2048 px wide; 2D address = (idx % 2048, idx / 2048).
    pub fn get_fps_point(i: u32) -> f32 {
        let ti = i / 4u32;
        let lane = i % 4u32;
        let t: Vec4f = texture_load(FPS_POINTS_TEX, vec2i(ti as i32, 0i32), 0i32);
        if lane == 0u32 {
            t.x()
        } else if lane == 1u32 {
            t.y()
        } else if lane == 2u32 {
            t.z()
        } else {
            t.w()
        }
    }

    pub fn get_curve(idx: u32) -> [Vec2f; 3] {
        let base = idx * 2u32;
        let a0 = base;
        let a1 = base + 1u32;
        let t0: Vec4f = texture_load(
            SLUG_CURVES_TEX,
            vec2i((a0 % 2048u32) as i32, (a0 / 2048u32) as i32),
            0i32,
        );
        let t1: Vec4f = texture_load(
            SLUG_CURVES_TEX,
            vec2i((a1 % 2048u32) as i32, (a1 / 2048u32) as i32),
            0i32,
        );
        [
            vec2f(t0.x(), t0.y()),
            vec2f(t0.z(), t0.w()),
            vec2f(t1.x(), t1.y()),
        ]
    }

    pub fn get_curve_index(idx: u32) -> u32 {
        let ti = idx / 4u32;
        let lane = idx % 4u32;
        let t: Vec4u = texture_load(
            SLUG_CURVE_INDICES_TEX,
            vec2i((ti % 2048u32) as i32, (ti / 2048u32) as i32),
            0i32,
        );
        if lane == 0u32 {
            t.x()
        } else if lane == 1u32 {
            t.y()
        } else if lane == 2u32 {
            t.z()
        } else {
            t.w()
        }
    }

    pub fn get_glyph(idx: u32) -> GlyphInfo {
        let base = idx * 5u32;
        let a0 = base;
        let a1 = base + 1u32;
        let a2 = base + 2u32;
        let a3 = base + 3u32;
        let a4 = base + 4u32;
        let t0: Vec4u = texture_load(
            SLUG_GLYPHS_TEX,
            vec2i((a0 % 2048u32) as i32, (a0 / 2048u32) as i32),
            0i32,
        );
        let t1: Vec4u = texture_load(
            SLUG_GLYPHS_TEX,
            vec2i((a1 % 2048u32) as i32, (a1 / 2048u32) as i32),
            0i32,
        );
        let t2: Vec4u = texture_load(
            SLUG_GLYPHS_TEX,
            vec2i((a2 % 2048u32) as i32, (a2 / 2048u32) as i32),
            0i32,
        );
        let t3: Vec4u = texture_load(
            SLUG_GLYPHS_TEX,
            vec2i((a3 % 2048u32) as i32, (a3 / 2048u32) as i32),
            0i32,
        );
        let t4: Vec4u = texture_load(
            SLUG_GLYPHS_TEX,
            vec2i((a4 % 2048u32) as i32, (a4 / 2048u32) as i32),
            0i32,
        );
        GlyphInfo {
            data: t0,
            vband: [
                t1.x(),
                t1.y(),
                t1.z(),
                t1.w(),
                t2.x(),
                t2.y(),
                t2.z(),
                t2.w(),
            ],
            hband: [
                t3.x(),
                t3.y(),
                t3.z(),
                t3.w(),
                t4.x(),
                t4.y(),
                t4.z(),
                t4.w(),
            ],
        }
    }

    pub fn get_run(idx: u32) -> SlugRunDesc {
        let t: Vec4f = texture_load(
            SLUG_RUNS_TEX,
            vec2i((idx % 2048u32) as i32, (idx / 2048u32) as i32),
            0i32,
        );
        SlugRunDesc {
            natural_advance: t.x(),
            natural_height: t.y(),
            glyph_offset: t.z() as u32,
            glyph_count: t.w() as u32,
        }
    }

    pub fn get_glyph_layout(idx: u32) -> SlugGlyphLayout {
        let base = idx * 3u32;
        let b0 = base;
        let b1 = base + 1u32;
        let b2 = base + 2u32;
        let t0: Vec4f = texture_load(
            SLUG_GLYPH_LAYOUT_TEX,
            vec2i((b0 % 2048u32) as i32, (b0 / 2048u32) as i32),
            0i32,
        );
        let t1: Vec4f = texture_load(
            SLUG_GLYPH_LAYOUT_TEX,
            vec2i((b1 % 2048u32) as i32, (b1 / 2048u32) as i32),
            0i32,
        );
        let t2: Vec4f = texture_load(
            SLUG_GLYPH_LAYOUT_TEX,
            vec2i((b2 % 2048u32) as i32, (b2 / 2048u32) as i32),
            0i32,
        );
        SlugGlyphLayout {
            screen_rect: t0,
            em_rect: t1,
            glyph_index: t2.x() as u32,
            _pad: [0u32; 3],
        }
    }

    // Identical logic to the Default variant; only the accessor fns above differ.
    pub fn slug_fetch_band_ranges(em_pos: Vec2f, glyph_idx: u32) -> Vec4u {
        const BAND_COUNT: f32 = 8.0;
        let g = get_glyph(glyph_idx);
        let d_xy = g.data.x();
        let d_wh = g.data.y();
        let bbox_min = vec2f(f32(i32(d_xy << 16u32) >> 16u32), f32(i32(d_xy) >> 16u32));
        let bbox_max = vec2f(f32(i32(d_wh << 16u32) >> 16u32), f32(i32(d_wh) >> 16u32));
        let size = max(bbox_max - bbox_min, vec2f(0.0001, 0.0001));
        let band_f = (em_pos - bbox_min) * (BAND_COUNT / size);
        let bx = u32(clamp(i32(band_f.x()), 0i32, i32(BAND_COUNT) - 1i32));
        let by = u32(clamp(i32(band_f.y()), 0i32, i32(BAND_COUNT) - 1i32));
        let h_packed = g.hband[by as usize];
        let v_packed = g.vband[bx as usize];
        vec4u(
            h_packed & 0x00FFFFFFu32,
            h_packed >> 24u32,
            v_packed & 0x00FFFFFFu32,
            v_packed >> 24u32,
        )
    }

    pub fn slug_render_glyph(em_pos: Vec2f, band_ranges: Vec4u, pixels_per_em: Vec2f) -> f32 {
        let mut xcov: f32 = 0.0;
        let mut xwgt: f32 = 0.0;
        let h_base = band_ranges.x();
        let h_count = band_ranges.y();
        let mut ci: u32 = 0;
        while ci < h_count {
            let curve_idx = get_curve_index(h_base + ci);
            let curve = get_curve(curve_idx);
            let p0 = curve[0] - em_pos;
            let p1 = curve[1] - em_pos;
            let p2 = curve[2] - em_pos;
            if max(max(p0.x(), p1.x()), p2.x()) * pixels_per_em.x() < -0.5 {
                break;
            }
            let code = slug_calc_root_code(p0.y(), p1.y(), p2.y());
            if code != 0u32 {
                let p12 = vec4f(p0.x(), p0.y(), p1.x(), p1.y());
                let r = slug_solve_horiz_poly(p12, p2) * pixels_per_em.x();
                if (code & 1u32) != 0u32 {
                    xcov += clamp(r.x() + 0.5, 0.0, 1.0);
                    xwgt = max(xwgt, clamp(1.0 - abs(r.x()) * 2.0, 0.0, 1.0));
                }
                if code > 1u32 {
                    xcov -= clamp(r.y() + 0.5, 0.0, 1.0);
                    xwgt = max(xwgt, clamp(1.0 - abs(r.y()) * 2.0, 0.0, 1.0));
                }
            }
            ci += 1;
        }
        let mut ycov: f32 = 0.0;
        let mut ywgt: f32 = 0.0;
        let v_base = band_ranges.z();
        let v_count = band_ranges.w();
        let mut ci2: u32 = 0;
        while ci2 < v_count {
            let curve_idx = get_curve_index(v_base + ci2);
            let curve = get_curve(curve_idx);
            let p0 = curve[0] - em_pos;
            let p1 = curve[1] - em_pos;
            let p2 = curve[2] - em_pos;
            if max(max(p0.y(), p1.y()), p2.y()) * pixels_per_em.y() < -0.5 {
                break;
            }
            let code = slug_calc_root_code(p0.x(), p1.x(), p2.x());
            if code != 0u32 {
                let p12 = vec4f(p0.x(), p0.y(), p1.x(), p1.y());
                let r = slug_solve_vert_poly(p12, p2) * pixels_per_em.y();
                if (code & 1u32) != 0u32 {
                    ycov -= clamp(r.x() + 0.5, 0.0, 1.0);
                    ywgt = max(ywgt, clamp(1.0 - abs(r.x()) * 2.0, 0.0, 1.0));
                }
                if code > 1u32 {
                    ycov += clamp(r.y() + 0.5, 0.0, 1.0);
                    ywgt = max(ywgt, clamp(1.0 - abs(r.y()) * 2.0, 0.0, 1.0));
                }
            }
            ci2 += 1;
        }
        slug_calc_coverage(xcov, ycov, xwgt, ywgt)
    }

    pub fn slugtext(px: Vec2f, rect: Vec4f, layout_flags: u32, run_idx: u32) -> f32 {
        if px.x() < rect.x() || px.x() > rect.z() || px.y() < rect.y() || px.y() > rect.w() {
            return 0.0;
        }
        let run = get_run(run_idx);
        if run.glyph_count == 0u32 {
            return 0.0;
        }
        let region = vec2f(rect.z() - rect.x(), rect.w() - rect.y());
        let v_bits = layout_flags & 0x3u32;
        let h_bits = (layout_flags >> 2u32) & 0x3u32;
        let adv = max(run.natural_advance, 0.001);
        let hgt = max(run.natural_height, 0.001);
        let scale_w = region.x() / adv;
        let scale_h = region.y() / hgt;
        let mut em_scale_x: f32 = 0.0;
        let mut em_scale_y: f32 = 0.0;
        if h_bits == 3u32 {
            em_scale_x = scale_w;
            em_scale_y = scale_h;
        } else {
            let s = min(scale_w, scale_h);
            em_scale_x = s;
            em_scale_y = s;
        }
        let scaled_advance = adv * em_scale_x;
        let mut h_off: f32 = 0.0;
        if h_bits == 1u32 || h_bits == 3u32 {
            h_off = 0.0;
        } else if h_bits == 2u32 {
            h_off = region.x() - scaled_advance;
        } else {
            h_off = (region.x() - scaled_advance) * 0.5;
        }
        let scaled_height = hgt * em_scale_y;
        let mut v_off: f32 = 0.0;
        if v_bits == 1u32 || h_bits == 3u32 {
            v_off = 0.0;
        } else if v_bits == 2u32 {
            v_off = region.y() - scaled_height;
        } else {
            v_off = (region.y() - scaled_height) * 0.5;
        }
        let origin = vec2f(rect.x() + h_off, rect.y() + v_off);
        let glyph_local = (px - origin) / vec2f(em_scale_x, em_scale_y);
        let g0 = get_glyph_layout(run.glyph_offset);
        let screen_size = g0.screen_rect.zw() - g0.screen_rect.xy();
        let em_size = abs(g0.em_rect.zw() - g0.em_rect.xy());
        let base_ppe = screen_size / max(em_size, vec2f(0.001, 0.001));
        let pixels_per_em = base_ppe * vec2f(em_scale_x, em_scale_y);
        let mut coverage: f32 = 0.0;
        let end = run.glyph_offset + run.glyph_count;
        let mut i = run.glyph_offset;
        while i < end {
            let g = get_glyph_layout(i);
            if glyph_local.x() < g.screen_rect.x() || glyph_local.x() > g.screen_rect.z() {
                i += 1;
                continue;
            }
            if glyph_local.y() < g.screen_rect.y() || glyph_local.y() > g.screen_rect.w() {
                i += 1;
                continue;
            }
            let t = (glyph_local - g.screen_rect.xy())
                / max(g.screen_rect.zw() - g.screen_rect.xy(), vec2f(0.001, 0.001));
            let em = mix(g.em_rect.xy(), g.em_rect.zw(), t);
            let band_ranges = slug_fetch_band_ranges(em, g.glyph_index);
            coverage = max(coverage, slug_render_glyph(em, band_ranges, pixels_per_em));
            i += 1;
        }
        coverage
    }

    pub fn draw_sparkline(plot_uv: Vec2f, min_fps: f32, max_fps: f32, line_width: f32) -> f32 {
        let count: u32 = 256u32;
        let mut dist: f32 = 1e9;
        let pad = line_width + 0.005;
        let mut i: u32 = 0u32;
        while i + 1u32 < count {
            let raw0 = get_fps_point(i);
            let raw1 = get_fps_point(i + 1u32);
            if raw0 > 0.0 && raw1 > 0.0 {
                let y0 = clamp(normalize_fps(raw0, min_fps, max_fps), pad, 1.0 - pad);
                let y1 = clamp(normalize_fps(raw1, min_fps, max_fps), pad, 1.0 - pad);
                let p0 = vec2f(plot_point_x(i, count), 1.0 - y0);
                let p1 = vec2f(plot_point_x(i + 1u32, count), 1.0 - y1);
                dist = update_min_dist(dist, plot_uv, p0, p1);
            }
            i += 1;
        }
        coverage_from_dist(dist, line_width)
    }

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let uv = input.uv;
        let params = get!(OVERLAY_PARAMS);
        let background = params.background_color;
        let text_color = params.text_color;
        let layout_flags = params.layout_flags;
        let alpha_discard = params.alpha_discard;
        let color_mode = params.color_mode;
        let node_size = params.node_size;
        let min_fps = params.min_fps;
        let max_fps = params.max_fps;
        let line_width = params.line_width;

        if in_plot_region(uv) {
            let plot_uv = to_plot_uv(uv);
            let coverage = draw_sparkline(plot_uv, min_fps, max_fps, line_width);
            stats_overlay_shade(coverage, alpha_discard, color_mode, text_color, background)
        } else {
            let text_uv = to_text_uv(uv);
            let text_frac = 1.0 - PLOT_FRAC;
            let text_w = node_size.x() * text_frac;
            let text_h = node_size.y();
            let px = text_uv * vec2f(text_w, text_h);
            let rect = vec4f(0.0, 0.0, text_w, text_h);
            let coverage = slugtext(px, rect, layout_flags, 0u32);
            stats_overlay_shade(coverage, alpha_discard, color_mode, text_color, background)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stats_overlay_helpers::*;
    use wgsl_rs::std::*;

    #[test]
    fn plot_frac_matches_pixel_ratio() {
        let expected = 100.0_f32 / 230.0;
        assert!(
            (PLOT_FRAC - expected).abs() < 1e-5,
            "PLOT_FRAC={PLOT_FRAC} expected {expected}"
        );
    }

    #[test]
    fn plot_frac_plus_text_frac_sums_to_one() {
        assert!((PLOT_FRAC + (1.0 - PLOT_FRAC) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn plot_region_left_origin() {
        assert!(in_plot_region(vec2f(0.0, 0.5)));
    }

    #[test]
    fn plot_region_at_frac_boundary_is_text() {
        assert!(!in_plot_region(vec2f(PLOT_FRAC, 0.5)));
    }

    #[test]
    fn plot_region_right_side_is_text() {
        assert!(!in_plot_region(vec2f(0.8, 0.5)));
    }

    #[test]
    fn to_plot_uv_at_split_is_one() {
        let p = to_plot_uv(vec2f(PLOT_FRAC, 0.0));
        assert!(
            (p.x() - 1.0).abs() < 1e-5,
            "x=PLOT_FRAC -> 1, got {}",
            p.x()
        );
    }

    #[test]
    fn to_text_uv_at_split_is_zero() {
        let t = to_text_uv(vec2f(PLOT_FRAC, 0.5));
        assert!(
            t.x().abs() < 1e-5,
            "x=PLOT_FRAC -> text_uv.x=0, got {}",
            t.x()
        );
    }

    #[test]
    fn to_text_uv_right_edge_is_one() {
        let t = to_text_uv(vec2f(1.0, 0.0));
        assert!(
            (t.x() - 1.0).abs() < 1e-5,
            "x=1.0 -> text_uv.x=1, got {}",
            t.x()
        );
    }

    #[test]
    fn normalize_fps_at_min_returns_zero() {
        assert!(normalize_fps(30.0, 30.0, 90.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_fps_at_max_returns_one() {
        assert!((normalize_fps(90.0, 30.0, 90.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_fps_zero_range_returns_half() {
        assert!((normalize_fps(60.0, 60.0, 60.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn plot_point_x_first_is_zero() {
        assert!(plot_point_x(0, 256).abs() < 1e-6);
    }

    #[test]
    fn plot_point_x_last_is_one() {
        assert!((plot_point_x(255, 256) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn stats_overlay_constants_sensible() {
        const { assert!(crate::STATS_OVERLAY_POINT_COUNT > 0) };
        const { assert!(crate::STATS_OVERLAY_HISTORY_SIZE > crate::STATS_OVERLAY_POINT_COUNT) };
    }
}
