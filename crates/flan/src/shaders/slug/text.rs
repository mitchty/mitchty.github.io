//! Slug text UiMaterial shader standalone slug text widget.
//!
//! Two GPU variants:
//!
//! * `slug_text_default`  - `@group(1)`, data in storage buffers.
//!   Use on native, WebGPU, and any target that supports `STORAGE_RESOURCE`.
//!
//! * `slug_text_texture`  - `@group(1)`, data packed into 2-D `rgba32`
//!   textures read with `textureLoad`. Use on WebGL2.
//!
//! Both modules expose a `WGSL_MODULE` constant and `wgsl_source()` via
//! `wgsl-rs`. Register the one you want in `ShadersPlugin` and pair it
//! with the matching Bevy `UiMaterial` type in `lib.rs`.
use wgsl_rs::wgsl;

// TODO: All this mod gating is mostly leftover from direct porting from wesl
// files. Need a better layout in pure rust lande even with #[wgsl] wrappers as
// lib fns.
#[wgsl]
pub mod slug_text_types {
    use wgsl_rs::std::*;

    /// 80-byte GPU glyph record.
    #[derive(Copy, Clone, Wgsl)]
    pub struct GlyphInfo {
        pub data: Vec4u,
        pub vband: [u32; 8],
        pub hband: [u32; 8],
    }

    /// Per-glyph layout: 48 bytes = 3 x `vec4<f32>`.
    /// `_pad` is `[u32; 3]` not `Vec3u` as `vec3<u32>` has 16-byte GPU
    /// alignment issues which bloats to 64 bytes and causes stride mismatches.
    // TODO: future me bytemuck/glam fix this bs I got side tracked and this is
    // "make it work" at best code.
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

    /// 96-byte uniform packed by `AsBindGroup::as_bind_group` in lib.rs:
    ///
    /// ```text
    /// offset  0: node_size      vec2<f32>
    /// offset  8: layout_flags   u32
    /// offset 12: alpha_discard  f32
    /// offset 16: text_color     vec4<f32>
    /// offset 32: local_to_clip  mat4x4<f32>
    /// ```
    #[derive(Copy, Clone, Wgsl)]
    pub struct SlugParamsUniform {
        pub node_size: Vec2f,
        pub layout_flags: u32,
        pub alpha_discard: f32,
        pub text_color: Vec4f,
        pub local_to_clip: Mat4f,
    }
}

/// Slug text UiMaterial shader.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  SlugParamsUniform               96bytes
/// binding(1)  storage  slug_curves        RuntimeArray<[Vec2f; 3]>
/// binding(2)  storage  slug_curve_indices RuntimeArray<u32>
/// binding(3)  storage  slug_glyphs        RuntimeArray<GlyphInfo>
/// binding(4)  storage  slug_runs          RuntimeArray<SlugRunDesc>
/// binding(5)  storage  slug_glyph_layout  RuntimeArray<SlugGlyphLayout>
/// ```
#[wgsl]
#[allow(unused_assignments)]
pub mod slug_text_default {
    use super::super::slug_helpers::*;
    use super::slug_text_types::*;
    use wgsl_rs::std::*;

    uniform!(group(1), binding(0), SLUG_PARAMS:        SlugParamsUniform);
    storage!(group(1), binding(1), SLUG_CURVES:        RuntimeArray<[Vec2f; 3]>);
    storage!(group(1), binding(2), SLUG_CURVE_INDICES: RuntimeArray<u32>);
    storage!(group(1), binding(3), SLUG_GLYPHS:        RuntimeArray<GlyphInfo>);
    storage!(group(1), binding(4), SLUG_RUNS:          RuntimeArray<SlugRunDesc>);
    storage!(group(1), binding(5), SLUG_GLYPH_LAYOUT:  RuntimeArray<SlugGlyphLayout>);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
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
        let h_base: u32 = band_ranges.x();
        let h_count: u32 = band_ranges.y();
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
        let v_base: u32 = band_ranges.z();
        let v_count: u32 = band_ranges.w();
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
        if h_bits == SLUG_LAYOUT_HFILL {
            em_scale_x = scale_w;
            em_scale_y = scale_h;
        } else if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HRIGHT {
            em_scale_x = scale_h;
            em_scale_y = scale_h;
        } else {
            let s = min(scale_w, scale_h);
            em_scale_x = s;
            em_scale_y = s;
        }
        let scaled_advance = adv * em_scale_x;
        let mut h_off: f32 = 0.0;
        if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HFILL {
            h_off = 0.0;
        } else if h_bits == SLUG_LAYOUT_HRIGHT {
            h_off = region.x() - scaled_advance;
        } else {
            h_off = (region.x() - scaled_advance) * 0.5;
        }
        let scaled_height = hgt * em_scale_y;
        let mut v_off: f32 = 0.0;
        if v_bits == SLUG_LAYOUT_VTOP || h_bits == SLUG_LAYOUT_HFILL {
            v_off = 0.0;
        } else if v_bits == SLUG_LAYOUT_VBOTTOM {
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

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let node_size = get!(SLUG_PARAMS).node_size;
        let px = input.uv * node_size;
        let rect = vec4f(0.0, 0.0, node_size.x(), node_size.y());
        let layout_flags = get!(SLUG_PARAMS).layout_flags;
        let color = get!(SLUG_PARAMS).text_color;
        let coverage = slugtext(px, rect, layout_flags, 0u32);
        color * coverage
    }
}

/// Slug text UiMaterial shader.
///
/// Data layout (`@group(1)`):
/// ```text
/// binding(0)  uniform  SlugParamsUniform            96bytes
/// binding(1)  texture  slug_curves_tex        rgba32float  2 texels/curve
/// binding(2)  texture  slug_curve_indices_tex rgba32uint   4 u32/texel
/// binding(3)  texture  slug_glyphs_tex        rgba32uint   5 texels/GlyphInfo
/// binding(4)  texture  slug_runs_tex          rgba32float  1 texel/SlugRunDesc
/// binding(5)  texture  slug_glyph_layout_tex  rgba32float  3 texels/SlugGlyphLayout
/// ```
/// All textures are 2048 px wide and as tall as needed.
#[wgsl]
#[allow(unused_assignments)]
pub mod slug_text_texture {
    use super::super::slug_helpers::*;
    use super::slug_text_types::*;
    use wgsl_rs::std::*;

    uniform!(group(1), binding(0), SLUG_PARAMS:             SlugParamsUniform);
    texture!(group(1), binding(1), SLUG_CURVES_TEX:         Texture2D<f32>);
    texture!(group(1), binding(2), SLUG_CURVE_INDICES_TEX:  Texture2D<u32>);
    texture!(group(1), binding(3), SLUG_GLYPHS_TEX:         Texture2D<u32>);
    texture!(group(1), binding(4), SLUG_RUNS_TEX:           Texture2D<f32>);
    texture!(group(1), binding(5), SLUG_GLYPH_LAYOUT_TEX:   Texture2D<f32>);

    pub struct FragmentInput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
    }

    // All textures are 2048 px wide, the 2D address = (idx % 2048, idx / 2048).

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
        let h_base: u32 = band_ranges.x();
        let h_count: u32 = band_ranges.y();
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
        let v_base: u32 = band_ranges.z();
        let v_count: u32 = band_ranges.w();
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
        if h_bits == SLUG_LAYOUT_HFILL {
            em_scale_x = scale_w;
            em_scale_y = scale_h;
        } else if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HRIGHT {
            em_scale_x = scale_h;
            em_scale_y = scale_h;
        } else {
            let s = min(scale_w, scale_h);
            em_scale_x = s;
            em_scale_y = s;
        }
        let scaled_advance = adv * em_scale_x;
        let mut h_off: f32 = 0.0;
        if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HFILL {
            h_off = 0.0;
        } else if h_bits == SLUG_LAYOUT_HRIGHT {
            h_off = region.x() - scaled_advance;
        } else {
            h_off = (region.x() - scaled_advance) * 0.5;
        }
        let scaled_height = hgt * em_scale_y;
        let mut v_off: f32 = 0.0;
        if v_bits == SLUG_LAYOUT_VTOP || h_bits == SLUG_LAYOUT_HFILL {
            v_off = 0.0;
        } else if v_bits == SLUG_LAYOUT_VBOTTOM {
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

    #[fragment]
    pub fn fragment(input: FragmentInput) -> Vec4f {
        let node_size = get!(SLUG_PARAMS).node_size;
        let px = input.uv * node_size;
        let rect = vec4f(0.0, 0.0, node_size.x(), node_size.y());
        let layout_flags = get!(SLUG_PARAMS).layout_flags;
        let color = get!(SLUG_PARAMS).text_color;
        let coverage = slugtext(px, rect, layout_flags, 0u32);
        color * coverage
    }
}
