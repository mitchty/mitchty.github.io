// Slug text renderer core.
//
// Ported from gabdube/rust-slug-wgpu and Eric Lengyel's Slug algorithm.
// Reference: https://jcgt.org/published/0006/02/02/
//
// Each glyph is a CPU-shaped quad. The vertex shader interpolates em-space
// coordinates across the quad. The fragment shader uses those coordinates plus
// the per-vertex glyph index to look up band data and evaluate coverage.
//
// Binding layout is in same slots on native/webgl, different resource within each path:
//
//   @binding(0) - SlugParams uniform        native/webgl
//   @binding(1) - curves                    storage array  OR texture_2d<f32>
//   @binding(2) - curve_indices             storage array  OR texture_2d<u32>
//   @binding(3) - glyphs                    storage array  OR texture_2d<u32>
//
// Group selection mirrors the rest of flan:
//   UI_MATERIAL  -> @group(1)
//   else         -> @group(0)   for headless wgpu test harness only for now.

import package::flan::shaders::lib::slug::types::{
    GlyphInfo,
    SlugGlyphLayout,
    SlugRunDesc,
    SlugParams,
    SLUG_TEX_WIDTH,
};
import package::flan::shaders::lib::slug::math::{
    slug_calc_root_code,
    slug_solve_horiz_poly,
    slug_solve_vert_poly,
    slug_calc_coverage,
};

@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(0) var<uniform>       slug_params:        SlugParams;
@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(1) var<storage, read> slug_curves:        array<array<vec2<f32>, 3>>;
@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(2) var<storage, read> slug_curve_indices: array<u32>;
@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(3) var<storage, read> slug_glyphs:        array<GlyphInfo>;
@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(4) var<storage, read> slug_runs:          array<SlugRunDesc>;
@if(UI_MATERIAL && !WEBGL)
@group(1) @binding(5) var<storage, read> slug_glyph_layout:  array<SlugGlyphLayout>;

@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(0) var<uniform>       slug_params:        SlugParams;
@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(1) var<storage, read> slug_curves:        array<array<vec2<f32>, 3>>;
@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(2) var<storage, read> slug_curve_indices: array<u32>;
@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(3) var<storage, read> slug_glyphs:        array<GlyphInfo>;
@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(4) var<storage, read> slug_runs:          array<SlugRunDesc>;
@if(!UI_MATERIAL && !WEBGL && !MATERIAL_3D)
@group(0) @binding(5) var<storage, read> slug_glyph_layout:  array<SlugGlyphLayout>;

@if(UI_MATERIAL && WEBGL)
@group(1) @binding(0) var<uniform>                slug_params:            SlugParams;
@if(UI_MATERIAL && WEBGL)
@group(1) @binding(1) var slug_curves_tex:        texture_2d<f32>;
@if(UI_MATERIAL && WEBGL)
@group(1) @binding(2) var slug_curve_indices_tex: texture_2d<u32>;
@if(UI_MATERIAL && WEBGL)
@group(1) @binding(3) var slug_glyphs_tex:        texture_2d<u32>;

@if(UI_MATERIAL && WEBGL)
@group(1) @binding(4) var slug_runs_tex:          texture_2d<f32>;

@if(UI_MATERIAL && WEBGL)
@group(1) @binding(5) var slug_glyph_layout_tex:  texture_2d<f32>;

// Non-UI webgl path (Material2d / headless test harness).
// Excluded when MATERIAL_3D is true to avoid duplicate binding declarations.
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(0) var<uniform>                slug_params:            SlugParams;
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(1) var slug_curves_tex:        texture_2d<f32>;
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(2) var slug_curve_indices_tex: texture_2d<u32>;
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(3) var slug_glyphs_tex:        texture_2d<u32>;
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(4) var slug_runs_tex:          texture_2d<f32>;
@if(!UI_MATERIAL && WEBGL && !MATERIAL_3D)
@group(0) @binding(5) var slug_glyph_layout_tex:  texture_2d<f32>;

// 3-D Material path - native uses storage buffers, WebGL uses data textures.
// Both land at group 3 (MATERIAL_BIND_GROUP_INDEX in Bevy 0.18).
// Binding 0 is declared in mesh3d.wesl to avoid duplicate declarations.
@if(MATERIAL_3D && !WEBGL)
@group(3) @binding(1) var<storage, read> slug_curves:        array<array<vec2<f32>, 3>>;
@if(MATERIAL_3D && !WEBGL)
@group(3) @binding(2) var<storage, read> slug_curve_indices: array<u32>;
@if(MATERIAL_3D && !WEBGL)
@group(3) @binding(3) var<storage, read> slug_glyphs:        array<GlyphInfo>;
@if(MATERIAL_3D && !WEBGL)
@group(3) @binding(4) var<storage, read> slug_runs:          array<SlugRunDesc>;
@if(MATERIAL_3D && !WEBGL)
@group(3) @binding(5) var<storage, read> slug_glyph_layout:  array<SlugGlyphLayout>;

@if(MATERIAL_3D && WEBGL)
@group(3) @binding(1) var slug_curves_tex:        texture_2d<f32>;
@if(MATERIAL_3D && WEBGL)
@group(3) @binding(2) var slug_curve_indices_tex: texture_2d<u32>;
@if(MATERIAL_3D && WEBGL)
@group(3) @binding(3) var slug_glyphs_tex:        texture_2d<u32>;
@if(MATERIAL_3D && WEBGL)
@group(3) @binding(4) var slug_runs_tex:          texture_2d<f32>;
@if(MATERIAL_3D && WEBGL)
@group(3) @binding(5) var slug_glyph_layout_tex:  texture_2d<f32>;


// Native is a direct array index.
// webgl is decode 2 rgba32float texels.
//   texel (base+0) = (p0.x, p0.y, p1.x, p1.y)
//   texel (base+1) = (p2.x, p2.y, 0, 0)
@if(!WEBGL)
fn slug_get_curve(idx: u32) -> array<vec2<f32>, 3> {
    return slug_curves[idx];
}

@if(WEBGL)
fn slug_get_curve(idx: u32) -> array<vec2<f32>, 3> {
    let base  = idx * 2u;
    let col0  = i32(base % SLUG_TEX_WIDTH);
    let row0  = i32(base / SLUG_TEX_WIDTH);
    let col1  = i32((base + 1u) % SLUG_TEX_WIDTH);
    let row1  = i32((base + 1u) / SLUG_TEX_WIDTH);
    let t0    = textureLoad(slug_curves_tex, vec2<i32>(col0, row0), 0);
    let t1    = textureLoad(slug_curves_tex, vec2<i32>(col1, row1), 0);
    var pts: array<vec2<f32>, 3>;
    pts[0] = t0.xy;
    pts[1] = t0.zw;
    pts[2] = t1.xy;
    return pts;
}

// Native is direct array index. W
// webgl is decode rgba32uint (4 u32s per texel).
@if(!WEBGL)
fn slug_get_curve_index(idx: u32) -> u32 {
    return slug_curve_indices[idx];
}

@if(WEBGL)
fn slug_get_curve_index(idx: u32) -> u32 {
    let texel_idx = idx / 4u;
    let lane      = idx % 4u;
    let col = i32(texel_idx % SLUG_TEX_WIDTH);
    let row = i32(texel_idx / SLUG_TEX_WIDTH);
    return textureLoad(slug_curve_indices_tex, vec2<i32>(col, row), 0)[lane];
}

// Native: direct struct index.
// webgl: decode 5 rgba32uint texels per glyph.
//   texel (base+0): data.xyzw
//   texel (base+1): vband[0..3]
//   texel (base+2): vband[4..7]
//   texel (base+3): hband[0..3]
//   texel (base+4): hband[4..7]
@if(!WEBGL)
fn slug_get_glyph(idx: u32) -> GlyphInfo {
    return slug_glyphs[idx];
}

@if(WEBGL)
fn slug_load_glyph_texel(base: u32, offset: u32) -> vec4<u32> {
    let abs_texel = base + offset;
    let col = i32(abs_texel % SLUG_TEX_WIDTH);
    let row = i32(abs_texel / SLUG_TEX_WIDTH);
    return textureLoad(slug_glyphs_tex, vec2<i32>(col, row), 0);
}

@if(WEBGL)
fn slug_get_glyph(idx: u32) -> GlyphInfo {
    let base = idx * 5u;
    var g: GlyphInfo;
    let t0 = slug_load_glyph_texel(base, 0u);
    let t1 = slug_load_glyph_texel(base, 1u);
    let t2 = slug_load_glyph_texel(base, 2u);
    let t3 = slug_load_glyph_texel(base, 3u);
    let t4 = slug_load_glyph_texel(base, 4u);
    g.data      = t0;
    g.vband[0u] = t1.x; g.vband[1u] = t1.y; g.vband[2u] = t1.z; g.vband[3u] = t1.w;
    g.vband[4u] = t2.x; g.vband[5u] = t2.y; g.vband[6u] = t2.z; g.vband[7u] = t2.w;
    g.hband[0u] = t3.x; g.hband[1u] = t3.y; g.hband[2u] = t3.z; g.hband[3u] = t3.w;
    g.hband[4u] = t4.x; g.hband[5u] = t4.y; g.hband[6u] = t4.z; g.hband[7u] = t4.w;
    return g;
}

// Each SlugGlyphLayout is 48 bytes = 3 x vec4<f32>.
//   texel 0: screen_rect  (xmin, ymin, xmax, ymax)  node-space pixels
//   texel 1: em_rect      (xmin_em, ymax_em, xmax_em, ymin_em)  y-flipped
//   texel 2: (glyph_index as f32, 0, 0, 0)

@if(!WEBGL)
fn slug_get_glyph_layout(idx: u32) -> SlugGlyphLayout {
    return slug_glyph_layout[idx];
}

@if(WEBGL)
fn slug_load_layout_texel(base: u32, off: u32) -> vec4<f32> {
    let abs = base + off;
    return textureLoad(slug_glyph_layout_tex,
        vec2<i32>(i32(abs % SLUG_TEX_WIDTH), i32(abs / SLUG_TEX_WIDTH)), 0);
}

@if(WEBGL)
fn slug_get_glyph_layout(idx: u32) -> SlugGlyphLayout {
    let base = idx * 3u;
    var g: SlugGlyphLayout;
    g.screen_rect  = slug_load_layout_texel(base, 0u);
    g.em_rect      = slug_load_layout_texel(base, 1u);
    let t2         = slug_load_layout_texel(base, 2u);
    g.glyph_index  = u32(t2.x);
    return g;
}

// Each SlugRunDesc is 16 bytes = 1 x vec4<f32> on native, 1 rgba32float texel on webgl.
//   x = natural_advance (f32)
//   y = natural_height  (f32)
//   z = glyph_offset    (u32 reinterpreted as f32 - use bitcast)
//   w = glyph_count     (u32 reinterpreted as f32 - use bitcast)

@if(!WEBGL)
fn slug_get_run_desc(run_idx: u32) -> SlugRunDesc {
    return slug_runs[run_idx];
}

@if(WEBGL)
fn slug_get_run_desc(run_idx: u32) -> SlugRunDesc {
    // 1 texel per run, the runs texture is a 1D rgba32float strip.
    let col = i32(run_idx % SLUG_TEX_WIDTH);
    let row = i32(run_idx / SLUG_TEX_WIDTH);
    let t   = textureLoad(slug_runs_tex, vec2<i32>(col, row), 0);
    var r: SlugRunDesc;
    r.natural_advance = t.x;
    r.natural_height  = t.y;
    // glyph_offset and glyph_count are uploaded as f32 values to avoid
    // denormal-float flushing on webgl drivers.
    // u32(f32(N)) is exact for all N < 2^24.
    r.glyph_offset    = u32(t.z);
    r.glyph_count     = u32(t.w);
    return r;
}

// PackedRange is bits[31:24] = count, bits[23:0] = offset into curve_indices[].
fn slug_band_offset(packed: u32) -> u32 { return packed & 0x00FFFFFFu; }
fn slug_band_count(packed: u32)  -> u32 { return packed >> 24u; }

// Given an em-space sample position and absolute glyph index that returns the band
// packed ranges for the horizontal and vertical sweeps as vec4<u32>:
//   .xy = (h_offset, h_count)
//   .zw = (v_offset, v_count)
fn slug_fetch_band_ranges(em_pos: vec2<f32>, glyph_idx: u32) -> vec4<u32> {
    const BAND_COUNT: f32 = 8.0;

    let g = slug_get_glyph(glyph_idx);

    // Unpack bbox values are i16 raw font units (descenders have negative y).
    // Uses shifts only to mimic bitfieldextract which es 3.0 doesn't have:
    //
    //   bits [0:16]  (low  half): shift u32 left 16 -> sign bit at MSB ->
    //                             reinterpret as i32 -> arithmetic right-shift 16
    //   bits [16:32] (high half): reinterpret u32 directly as i32 ->
    //                             arithmetic right-shift 16
    let d_xy = g.data.x;
    let d_wh = g.data.y;
    let bbox_min = vec2<f32>(
        f32(i32(d_xy << 16u) >> 16u),
        f32(i32(d_xy) >> 16u),
    );
    let bbox_max = vec2<f32>(
        f32(i32(d_wh << 16u) >> 16u),
        f32(i32(d_wh) >> 16u),
    );

    let size = max(bbox_max - bbox_min, vec2<f32>(0.0001));
    let band_f = (em_pos - bbox_min) * (BAND_COUNT / size);
    let band   = vec2<u32>(clamp(
        vec2<i32>(i32(band_f.x), i32(band_f.y)),
        vec2<i32>(0),
        vec2<i32>(i32(BAND_COUNT) - 1),
    ));

    let h_packed = g.hband[band.y];
    let v_packed = g.vband[band.x];

    return vec4<u32>(
        slug_band_offset(h_packed), slug_band_count(h_packed),
        slug_band_offset(v_packed), slug_band_count(v_packed),
    );
}

// Evaluate coverage at `em_pos` given band ranges from slug_fetch_band_ranges.
// `pixels_per_em` must be computed OUTSIDE of a loop due to fwidth restriction on webgl.
fn slug_render(em_pos: vec2<f32>, band_ranges: vec4<u32>, pixels_per_em: vec2<f32>) -> f32 {
    var xcov = 0.0;
    var xwgt = 0.0;

    let h_base:  u32 = band_ranges.x;
    let h_count: u32 = band_ranges.y;

    for (var ci = 0u; ci < h_count; ci++) {
        let abs_curve_idx = slug_get_curve_index(h_base + ci);
        let curve         = slug_get_curve(abs_curve_idx);
        let p0 = curve[0] - em_pos;
        let p1 = curve[1] - em_pos;
        let p2 = curve[2] - em_pos;

        if max(max(p0.x, p1.x), p2.x) * pixels_per_em.x < -0.5 { break; }

        let code = slug_calc_root_code(p0.y, p1.y, p2.y);
        if code != 0u {
            let p12 = vec4<f32>(p0.x, p0.y, p1.x, p1.y);
            let r   = slug_solve_horiz_poly(p12, p2) * pixels_per_em.x;
            if (code & 1u) != 0u {
                xcov += clamp(r.x + 0.5, 0.0, 1.0);
                xwgt  = max(xwgt, clamp(1.0 - abs(r.x) * 2.0, 0.0, 1.0));
            }
            if code > 1u {
                xcov -= clamp(r.y + 0.5, 0.0, 1.0);
                xwgt  = max(xwgt, clamp(1.0 - abs(r.y) * 2.0, 0.0, 1.0));
            }
        }
    }

    var ycov = 0.0;
    var ywgt = 0.0;

    let v_base:  u32 = band_ranges.z;
    let v_count: u32 = band_ranges.w;

    for (var ci = 0u; ci < v_count; ci++) {
        let abs_curve_idx = slug_get_curve_index(v_base + ci);
        let curve         = slug_get_curve(abs_curve_idx);
        let p0 = curve[0] - em_pos;
        let p1 = curve[1] - em_pos;
        let p2 = curve[2] - em_pos;

        if max(max(p0.y, p1.y), p2.y) * pixels_per_em.y < -0.5 { break; }

        let code = slug_calc_root_code(p0.x, p1.x, p2.x);
        if code != 0u {
            let p12 = vec4<f32>(p0.x, p0.y, p1.x, p1.y);
            let r   = slug_solve_vert_poly(p12, p2) * pixels_per_em.y;
            if (code & 1u) != 0u {
                ycov -= clamp(r.x + 0.5, 0.0, 1.0);
                ywgt  = max(ywgt, clamp(1.0 - abs(r.x) * 2.0, 0.0, 1.0));
            }
            if code > 1u {
                ycov += clamp(r.y + 0.5, 0.0, 1.0);
                ywgt  = max(ywgt, clamp(1.0 - abs(r.y) * 2.0, 0.0, 1.0));
            }
        }
    }

    return slug_calc_coverage(xcov, ycov, xwgt, ywgt);
}

// These mirror the Rust `Layout` bitfield in layout.rs.
// Bits [1:0] = Vertical,   bits [3:2] = Horizontal.
//
//   Vertical:   Center=0  Top=1    Bottom=2  Unused=3
//   Horizontal: Center=0  Left=1   Right=2   Fill=3

const SLUG_LAYOUT_VCENTER: u32 = 0u;
const SLUG_LAYOUT_VTOP:    u32 = 1u;
const SLUG_LAYOUT_VBOTTOM: u32 = 2u;

const SLUG_LAYOUT_HCENTER: u32 = 0u;
const SLUG_LAYOUT_HLEFT:   u32 = 1u;
const SLUG_LAYOUT_HRIGHT:  u32 = 2u;
const SLUG_LAYOUT_HFILL:   u32 = 3u;

// Pre-packed constants for the common single-call convenience wrappers.
// Encode: layout = (h_bits << 2u) | v_bits
const SLUG_LAYOUT_CENTER:       u32 = 0u;        // H=Center V=Center
const SLUG_LAYOUT_LEFT:         u32 = 0x04u;     // H=Left   V=Center
const SLUG_LAYOUT_RIGHT:        u32 = 0x08u;     // H=Right  V=Center
const SLUG_LAYOUT_FILL:         u32 = 0x0Cu;     // H=Fill   V=Center
const SLUG_LAYOUT_TOP_LEFT:     u32 = 0x05u;     // H=Left   V=Top
const SLUG_LAYOUT_TOP_CENTER:   u32 = 0x01u;     // H=Center V=Top
const SLUG_LAYOUT_TOP_RIGHT:    u32 = 0x09u;     // H=Right  V=Top
const SLUG_LAYOUT_BOTTOM_LEFT:  u32 = 0x06u;     // H=Left   V=Bottom
const SLUG_LAYOUT_BOTTOM_CENTER:u32 = 0x02u;     // H=Center V=Bottom
const SLUG_LAYOUT_BOTTOM_RIGHT: u32 = 0x0Au;     // H=Right  V=Bottom

// slugtext() core layout-aware text drawing function
//
// Returns a scalar coverage value in [0, 1] for the current fragment.
// Color and compositing are the caller's responsibility for composition reasons.
//
// Parameters:
//   px      - current fragment position in node-space pixels (uv * node_size)
//   rect    - sub-region of the node to draw into: [x0, y0, x1, y1] in pixels.
//             Use (0, 0, node_size.x, node_size.y) for the full node.
//   layout_flags - 4-bit packed Layout value (use SLUG_LAYOUT_* constants above)
//   run_idx      - index into slug_runs[]; identifies which text run to render
//
// The function reads slug_runs[run_idx] to get the natural size and glyph
// range, then scales and anchors the run into `rect` according to `layout_flags`.
//
// pixels_per_em is derived from the scaled glyph dimensions.
fn slugtext(px: vec2<f32>, rect: vec4<f32>, layout_flags: u32, run_idx: u32) -> f32 {
    // pixel outside this uv range.
    if px.x < rect.x || px.x > rect.z ||
       px.y < rect.y || px.y > rect.w { return 0.0; }

    let run = slug_get_run_desc(run_idx);
    if run.glyph_count == 0u { return 0.0; }

    let region = rect.zw - rect.xy;  // (width, height) of the sub-region

    // Decode layout bits.
    let v_bits = layout_flags & 0x3u;
    let h_bits = (layout_flags >> 2u) & 0x3u;

    // Compute em_scale: how many pixels correspond to one natural-origin pixel.
    // natural_advance / natural_height are the run's dimensions at font_size.
    let adv = max(run.natural_advance, 0.001);
    let hgt = max(run.natural_height,  0.001);
    let scale_w = region.x / adv;
    let scale_h = region.y / hgt;

    var em_scale_x: f32;
    var em_scale_y: f32;
    if h_bits == SLUG_LAYOUT_HFILL {
        // Fill means each axis scales independently - aspect ratio abandoned
        // and the font size is not a concern.
        em_scale_x = scale_w;
        em_scale_y = scale_h;
    } else if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HRIGHT {
        // Fill-height will uniform scale from vertical fit only.
        em_scale_x = scale_h;
        em_scale_y = scale_h;
    } else {
        // Center to uniform scale, text always fully visible.
        let s = min(scale_w, scale_h);
        em_scale_x = s;
        em_scale_y = s;
    }

    let scaled_advance = adv * em_scale_x;
    var h_off: f32;
    if h_bits == SLUG_LAYOUT_HLEFT || h_bits == SLUG_LAYOUT_HFILL {
        h_off = 0.0;
    } else if h_bits == SLUG_LAYOUT_HRIGHT {
        h_off = region.x - scaled_advance;
    } else {
        // Center is default/fallback
        h_off = (region.x - scaled_advance) * 0.5;
    }

    let scaled_height = hgt * em_scale_y;
    var v_off: f32;
    if v_bits == SLUG_LAYOUT_VTOP || h_bits == SLUG_LAYOUT_HFILL {
        v_off = 0.0;
    } else if v_bits == SLUG_LAYOUT_VBOTTOM {
        v_off = region.y - scaled_height;
    } else {
        // Center is default/fallback
        v_off = (region.y - scaled_height) * 0.5;
    }

    // glyph_local_px is in the same coordinate space as screen_rect values
    // produced by shape() (natural origin, no layout applied).
    let origin       = rect.xy + vec2<f32>(h_off, v_off);
    let glyph_local  = (px - origin) / vec2<f32>(em_scale_x, em_scale_y);

    // Derive from the first glyph's screen_rect / em_rect ratio, then multiply
    // by em_scale to account for the layout scaling applied above.
    // Avoids fwidth() as its broken for clamped UV outside a glyph on webgl.
    // TODO: When webgl gets dumped fwidth() forever....
    let g0          = slug_get_glyph_layout(run.glyph_offset);
    let screen_size = g0.screen_rect.zw - g0.screen_rect.xy;
    let em_size     = abs(g0.em_rect.zw - g0.em_rect.xy);
    let base_ppe    = screen_size / max(em_size, vec2<f32>(0.001));
    let pixels_per_em = base_ppe * vec2<f32>(em_scale_x, em_scale_y);

    var coverage = 0.0;
    let end = run.glyph_offset + run.glyph_count;
    for (var i = run.glyph_offset; i < end; i++) {
        let g = slug_get_glyph_layout(i);

        if glyph_local.x < g.screen_rect.x || glyph_local.x > g.screen_rect.z { continue; }
        if glyph_local.y < g.screen_rect.y || glyph_local.y > g.screen_rect.w { continue; }

        // Interpolate em coordinate as em_rect is y-flipped ref types.wesl for y.
        let t  = (glyph_local - g.screen_rect.xy)
               / max(g.screen_rect.zw - g.screen_rect.xy, vec2<f32>(0.001));
        let em = mix(g.em_rect.xy, g.em_rect.zw, t);

        let band_ranges = slug_fetch_band_ranges(em, g.glyph_index);
        coverage = max(coverage, slug_render(em, band_ranges, pixels_per_em));
    }

    return coverage;
}

// Lazy wrappers around ^^^
fn slugtext_center(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_CENTER, run_idx);
}
fn slugtext_left(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_LEFT, run_idx);
}
fn slugtext_right(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_RIGHT, run_idx);
}
fn slugtext_fill(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_FILL, run_idx);
}
fn slugtext_top_left(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_TOP_LEFT, run_idx);
}
fn slugtext_top_center(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_TOP_CENTER, run_idx);
}
fn slugtext_top_right(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_TOP_RIGHT, run_idx);
}
fn slugtext_bottom_left(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_BOTTOM_LEFT, run_idx);
}
fn slugtext_bottom_center(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_BOTTOM_CENTER, run_idx);
}
fn slugtext_bottom_right(px: vec2<f32>, rect: vec4<f32>, run_idx: u32) -> f32 {
    return slugtext(px, rect, SLUG_LAYOUT_BOTTOM_RIGHT, run_idx);
}
