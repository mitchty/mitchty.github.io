// Slug algorithm type definitions
//
// Three bindings hold all font data:
//   curves[]        - quadratic Bezier control points (24 bytes / curve)
//   curve_indices[] - band spatial index (u32 per entry)
//   glyphs[]        - per-glyph metadata (80 bytes / glyph = 5 x vec4<u32>)
//
// WebGL2 data-texture constants.
// Width must match SLUG_TEX_WIDTH in lib.rs.
// Height is computed at runtime as ceil(element_count / elements_per_row).
//
// curves texture:   texture_2d<f32> rgba32float
//   2 texels per curve: texel0=(p0.x,p0.y,p1.x,p1.y), texel1=(p2.x,p2.y,0,0)
//   elements_per_row = SLUG_TEX_WIDTH / 2 curves per row
//
// curve_indices texture: texture_2d<u32> rgba32uint
//   4 u32s per texel, elements_per_row = SLUG_TEX_WIDTH * 4
//
// glyphs texture: texture_2d<u32> rgba32uint
//   5 texels per glyph (80 bytes = 5 x vec4<u32>)
//   elements_per_row = SLUG_TEX_WIDTH / 5 glyphs per row

const SLUG_TEX_WIDTH: u32 = 2048u;

// 80-byte GPU layout for one glyph, stored as 5 x vec4<u32>.
//
// slot 0:  data.x = (xmin as u16) | ((ymin as u16) << 16)   bbox low corner
//          data.y = (xmax as u16) | ((ymax as u16) << 16)   bbox high corner
//          data.z = curves_start  (absolute into curves[])
//          data.w = curves_end    (exclusive)
// slot 1:  vband[0..3]  vertical band packed ranges
// slot 2:  vband[4..7]
// slot 3:  hband[0..3]  horizontal band packed ranges
// slot 4:  hband[4..7]
//
// Each packed range: bits[31:24] = count, bits[23:0] = offset into curve_indices[].
struct GlyphInfo {
    data:  vec4<u32>,
    vband: array<u32, 8>,
    hband: array<u32, 8>,
}

// 48 bytes = 3 x vec4<f32>.
//
// screen_rect: [xmin, ymin, xmax, ymax] in natural-origin pixels.
//   "Natural origin" means shape() always outputs from (0, ascender_px) -
//   no layout transform is applied on the CPU. The shader applies scale and
//   offset at draw time based on the rect and layout passed to slugtext().
// em_rect:     [xmin_em, ymax_em, xmax_em, ymin_em] - y is FLIPPED so that
//   mix(em_rect.xy, em_rect.zw, t) gives the correct em coordinate since
//   font y increases upward while screen y increases downward.
// glyph_index: absolute index into slug_glyphs[] / slug_glyphs_tex.
//
// IMPORTANT: _pad must be array<u32, 3> NOT vec3<u32>.
// vec3<u32> has 16-byte GPU alignment, which pushes _pad to offset 48
// and bloats the struct to 64 bytes causing a stride mismatch where only
// every 4th glyph (where 4x48 = 3x64) reads the correct input data.
// array<u32, 3> has 4-byte alignment, sits at offset 36, total = 48 bytes.
struct SlugGlyphLayout {
    screen_rect:  vec4<f32>,
    em_rect:      vec4<f32>,
    glyph_index:  u32,
    _pad:         array<u32, 3>,
}

// 16 bytes = 1 x vec4<f32>. Fits in a single rgba32float texel on WebGL.
//
// natural_advance: total advance width of the run at font_size (pixels).
//   Used by the layout math to compute em_scale when fitting text into a rect.
// natural_height:  (ascender − descender) x scale at font_size (pixels).
// glyph_offset:    first index into slug_glyph_layout[] for this run.
// glyph_count:     number of SlugGlyphLayout entries in this run.
//
// sub_rect and layout are NOT stored here - they are slugtext() call-site
// parameters. The same run can be drawn with different rects and alignments.
struct SlugRunDesc {
    natural_advance: f32,
    natural_height:  f32,
    glyph_offset:    u32,
    glyph_count:     u32,        }

// 32 bytes - must match the packed buffer built in lib.rs exactly.
//
// This is NOT a slug lib concern it is the UiMaterial's uniform that
// ui_text.wesl reads before calling into the slug lib.
//
// Layout:
//   offset  0: node_size    vec2<f32>  - resolved node pixel size
//   offset  8: layout_flags u32        - 4-bit packed Layout bitfield (layout.rs)
//   offset 12: _pad         u32        - alignment padding
//   offset 16: text_color   vec4<f32>  - RGBA linear; applied at the call site
//
// layout_flags uses SLUG_LAYOUT_* constants from render.wesl.
// text_color is here because for the simple single-string
// UiMaterial case it is a per-material property. Multi-string / custom shaders
// apply color at their own call site with no struct involvement.
struct SlugParams {
    node_size:    vec2<f32>,
    layout_flags: u32,
    _pad:         u32,
    text_color:   vec4<f32>,
}
