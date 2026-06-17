//! Slug 3D text Material shader, per-glyph-quad vertex + fragment path for "etruded" 3d mesheses.
//!
//! Two GPU variants:
//!
//! * `slug_text3d_default`  - `@group(3)`, data in storage buffers.
//!   Use on native, WebGPU, and any target that supports `STORAGE_RESOURCE`.
//!
//! * `slug_text3d_texture`  - `@group(3)`, data packed into 2-D `rgba32`
//!   textures read with `textureLoad`. Use on WebGL2.
//!
//! Both modules expose a `WGSL_MODULE` constant and `wgsl_source()` via
//! `wgsl-rs`. Register the one you want in `ShadersPlugin` and pair it
//! with the matching Bevy `Material` type in `lib.rs`.
//!
//! Unlike the UiMaterial text path, the 3D path operates at the per-glyph-quad
//! level: the vertex shader unpacks em-coordinates from the packed glyph
//! attribute and the fragment shader uses `fwidth` for `pixels_per_em`.
//!
//! This is basically a bevy specific Mesh setup which differs from the shader
//! slugtext() fragment fn approach but reuses all the underlying logic for 3d
//! meshes.
use wgsl_rs::wgsl;

#[wgsl]
pub mod slug_text3d_types {
    use wgsl_rs::std::*;

    /// 80-byte GPU glyph record (5 x `vec4<u32>`).
    #[derive(Copy, Clone, Wgsl)]
    pub struct GlyphInfo {
        pub data: Vec4u,
        pub vband: [u32; 8],
        pub hband: [u32; 8],
    }

    /// 96-byte uniform:
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

/// Slug 3D text Material shader - native / WebGPU path.
///
/// Data layout (`@group(3)`):
/// ```text
/// binding(0)  uniform  SlugParamsUniform               (96bytes)
/// binding(1)  storage  slug_curves        RuntimeArray<[Vec2f; 3]>
/// binding(2)  storage  slug_curve_indices RuntimeArray<u32>
/// binding(3)  storage  slug_glyphs        RuntimeArray<GlyphInfo>
/// ```
#[wgsl]
pub mod slug_text3d_default {
    use super::super::slug_helpers::*;
    use super::slug_text3d_types::*;
    use wgsl_rs::std::*;

    uniform!(group(3), binding(0), SLUG_PARAMS:        SlugParamsUniform);
    storage!(group(3), binding(1), SLUG_CURVES:        RuntimeArray<[Vec2f; 3]>);
    storage!(group(3), binding(2), SLUG_CURVE_INDICES: RuntimeArray<u32>);
    storage!(group(3), binding(3), SLUG_GLYPHS:        RuntimeArray<GlyphInfo>);

    pub fn get_curve(idx: u32) -> [Vec2f; 3] {
        get!(SLUG_CURVES)[idx as usize]
    }

    pub fn get_curve_index(idx: u32) -> u32 {
        get!(SLUG_CURVE_INDICES)[idx as usize]
    }

    pub fn get_glyph(idx: u32) -> GlyphInfo {
        get!(SLUG_GLYPHS)[idx as usize]
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

    #[derive(Copy, Clone)]
    pub struct VertexInput {
        #[location(0)]
        pub pos: Vec4f,
        #[location(1)]
        pub glyph: Vec2u,
        #[location(2)]
        pub color: Vec4f,
    }

    pub struct VertexOutput {
        #[builtin(position)]
        pub clip_pos: Vec4f,
        #[location(0)]
        pub color: Vec4f,
        #[location(1)]
        pub em_pos: Vec2f,
        #[location(2)]
        #[interpolate(flat)]
        pub glyph_index: u32,
    }

    #[vertex]
    pub fn vertex(vin: VertexInput) -> VertexOutput {
        let clip_pos: Vec4f =
            get!(SLUG_PARAMS).local_to_clip * vec4f(vin.pos.x(), vin.pos.y(), vin.pos.z(), 1.0);
        let color: Vec4f = get!(SLUG_PARAMS).text_color;
        // Em-coordinate unpack: high 16 bits = em_x, low 16 bits = em_y.
        let packed = vin.glyph.x();
        let em_x_i16 = i32(packed) >> 16u32;
        let em_y_i16 = i32(packed << 16u32) >> 16u32;
        let em_pos = vec2f(f32(em_x_i16), f32(em_y_i16));
        let glyph_index = vin.glyph.y();
        VertexOutput {
            clip_pos,
            color,
            em_pos,
            glyph_index,
        }
    }

    #[fragment]
    pub fn fragment(input: VertexOutput) -> Vec4f {
        let ems_per_pixel = fwidth(input.em_pos);
        let pixels_per_em = vec2f(
            select(0.0, 1.0 / ems_per_pixel.x(), ems_per_pixel.x() > 1e-10),
            select(0.0, 1.0 / ems_per_pixel.y(), ems_per_pixel.y() > 1e-10),
        );
        let band_ranges = slug_fetch_band_ranges(input.em_pos, input.glyph_index);
        let coverage = slug_render_glyph(input.em_pos, band_ranges, pixels_per_em);
        let out_alpha = input.color.w() * coverage;
        if out_alpha < 0.01 {
            discard!();
        }
        vec4f(input.color.x(), input.color.y(), input.color.z(), out_alpha)
    }
}

/// Slug 3D text Material shader.
///
/// Data layout (`@group(3)`):
/// ```text
/// binding(0)  uniform  SlugParamsUniform            (96bytes)
/// binding(1)  texture  slug_curves_tex        rgba32float  2 texels/curve
/// binding(2)  texture  slug_curve_indices_tex rgba32uint   4 u32/texel
/// binding(3)  texture  slug_glyphs_tex        rgba32uint   5 texels/GlyphInfo
/// ```
/// All textures are 2048 px wide and as tall as needed.
#[wgsl]
pub mod slug_text3d_texture {
    use super::super::slug_helpers::*;
    use super::slug_text3d_types::*;
    use wgsl_rs::std::*;

    uniform!(group(3), binding(0), SLUG_PARAMS:            SlugParamsUniform);
    texture!(group(3), binding(1), SLUG_CURVES_TEX:        Texture2D<f32>);
    texture!(group(3), binding(2), SLUG_CURVE_INDICES_TEX: Texture2D<u32>);
    texture!(group(3), binding(3), SLUG_GLYPHS_TEX:        Texture2D<u32>);

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

    #[derive(Copy, Clone)]
    pub struct VertexInput {
        #[location(0)]
        pub pos: Vec4f,
        #[location(1)]
        pub glyph: Vec2u,
        #[location(2)]
        pub color: Vec4f,
    }

    pub struct VertexOutput {
        #[builtin(position)]
        pub clip_pos: Vec4f,
        #[location(0)]
        pub color: Vec4f,
        #[location(1)]
        pub em_pos: Vec2f,
        #[location(2)]
        #[interpolate(flat)]
        pub glyph_index: u32,
    }

    #[vertex]
    pub fn vertex(vin: VertexInput) -> VertexOutput {
        let clip_pos: Vec4f =
            get!(SLUG_PARAMS).local_to_clip * vec4f(vin.pos.x(), vin.pos.y(), vin.pos.z(), 1.0);
        let color: Vec4f = get!(SLUG_PARAMS).text_color;
        let packed = vin.glyph.x();
        let em_x_i16 = i32(packed) >> 16u32;
        let em_y_i16 = i32(packed << 16u32) >> 16u32;
        let em_pos = vec2f(f32(em_x_i16), f32(em_y_i16));
        let glyph_index = vin.glyph.y();
        VertexOutput {
            clip_pos,
            color,
            em_pos,
            glyph_index,
        }
    }

    #[fragment]
    pub fn fragment(input: VertexOutput) -> Vec4f {
        let ems_per_pixel = fwidth(input.em_pos);
        let pixels_per_em = vec2f(
            select(0.0, 1.0 / ems_per_pixel.x(), ems_per_pixel.x() > 1e-10),
            select(0.0, 1.0 / ems_per_pixel.y(), ems_per_pixel.y() > 1e-10),
        );
        let band_ranges = slug_fetch_band_ranges(input.em_pos, input.glyph_index);
        let coverage = slug_render_glyph(input.em_pos, band_ranges, pixels_per_em);
        let out_alpha = input.color.w() * coverage;
        if out_alpha < 0.01 {
            discard!();
        }
        vec4f(input.color.x(), input.color.y(), input.color.z(), out_alpha)
    }
}
