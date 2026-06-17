// Slug font atlas
//
// Parses TTF/OTF fonts via ttf-parser and produces GPU-ready buffers consumed
// by the slug WESL shaders. Follows the gabdube/rust-slug-wgpu architecture:
//
//   - Per-glyph vertex quads carry screen-space + em-space coords + glyph index.
//   - Text layout (advances, line breaks) is done entirely on the CPU in shape().
//   - Three tightly-packed storage buffers / data textures hold all font data:
//       curves[]        - raw quadratic Bezier control points
//       curve_indices[] - band spatial index (absolute offsets into curves[])
//       glyphs[]        - per-glyph metadata (bbox, band packed ranges)
//   - The frame atlas is a compact subset of the CPU cache: only glyphs
//     visible in the current frame are packed, with offsets rewritten to be
//     absolute in the compacted layout.
//
// Algorithm: Eric Lengyel, "GPU-Accelerated Path Rendering" (JCGT 2017)
// https://jcgt.org/published/0006/02/02/
//
// Em-space coordinates throughout this file are raw font units (i16 / f32),
// NOT normalized to [0..1]. Normalization happened in the old API; the new
// approach matches gabdube and the shader, which use raw font units directly.

use std::collections::HashMap;
use ttf_parser::{Face, GlyphId, OutlineBuilder, Rect};

/// Spatial bands per axis. 8 gives a good granularity/memory tradeoff for
/// most fonts. Must match the WESL shader constant.
pub const DEFAULT_BAND_COUNT: usize = 8;

/// Tiny perpendicular offset used when encoding a line as a degenerate quad.
/// Must be nonzero so the crossing test sees a non-collinear curve.
const LINE_EPSILON: f32 = 0.125;

/// Opaque handle to a registered font inside [`SlugAtlas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FontId(pub u32);

/// Outline format of a registered font.
///
/// | Format    | Outer contour | Hole/counter |
/// |-----------|---------------|--------------|
/// | TrueType  | CW  (negative shoelace area) | CCW (positive) |
/// | Cff       | CCW (positive shoelace area) | CW  (negative) |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineType {
    /// TrueType `glyf` outlines outer contours are CW in y up.
    TrueType,
    /// PostScript style CFF/CFF2 outlines outer contours are CCW in y up.
    Cff,
}

/// One quadratic bezier in raw font units.
#[derive(Clone, Copy, Debug)]
pub struct QuadCurve {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
}

impl QuadCurve {
    #[inline]
    fn max_x(&self) -> f32 {
        self.p0[0].max(self.p1[0]).max(self.p2[0])
    }
    #[inline]
    fn max_y(&self) -> f32 {
        self.p0[1].max(self.p1[1]).max(self.p2[1])
    }
    #[inline]
    fn bbox(&self) -> [f32; 4] {
        [
            self.p0[0].min(self.p1[0]).min(self.p2[0]),
            self.p0[1].min(self.p1[1]).min(self.p2[1]),
            self.p0[0].max(self.p1[0]).max(self.p2[0]),
            self.p0[1].max(self.p1[1]).max(self.p2[1]),
        ]
    }
}

/// A band range packed as `(count << 24) | offset24`.
/// `offset` is an absolute index into `curve_indices[]`.
/// `count`  is how many entries the band contains.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct PackedRange(u32);

impl PackedRange {
    fn new(offset: u32, count: u32) -> Self {
        debug_assert!(offset < (1 << 24), "band offset overflows 24 bits");
        debug_assert!(count < (1 << 8), "band count overflows 8 bits");
        PackedRange((count << 24) | offset)
    }
    #[inline]
    fn offset(self) -> u32 {
        self.0 & 0x00FF_FFFF
    }
    #[inline]
    fn count(self) -> u32 {
        self.0 >> 24
    }
}

/// All data for one glyph, extracted once from the TTF and kept forever.
/// Curves are in raw font units. Band offsets are *local* (0-based within this
/// glyph's own curve/index arrays). They are made absolute during frame-atlas
/// compaction.
#[derive(Clone, Debug)]
pub struct CachedGlyph {
    /// Bounding box in raw font units.
    pub bbox: [i16; 4], // [xmin, ymin, xmax, ymax]
    /// Bezier curves for this glyph in font units.
    pub curves: Vec<QuadCurve>,
    /// Start indices into `curves` for each contour. Used for side wall
    /// extrusion so that it can just iterate each contour stupidly.
    pub contour_starts: Vec<usize>,
    /// Flat array of curve-local indices into `curves`.
    /// All band packed ranges index into this array (local offsets).
    pub local_curve_indices: Vec<u32>,
    /// Vertical band packed ranges (offset into `local_curve_indices`).
    pub(crate) vband: [PackedRange; DEFAULT_BAND_COUNT],
    /// Horizontal band packed ranges (offset into `local_curve_indices`).
    pub(crate) hband: [PackedRange; DEFAULT_BAND_COUNT],
}

impl CachedGlyph {
    /// Iterate over each contour slice of a Slug `QuadCurve`.
    pub fn contours(&self) -> impl Iterator<Item = &[QuadCurve]> {
        self.contour_starts.iter().enumerate().map(|(i, &start)| {
            let end = self
                .contour_starts
                .get(i + 1)
                .copied()
                .unwrap_or(self.curves.len());
            &self.curves[start..end]
        })
    }
}

/// GPU layout for one glyph entry in the atlas storage buffer / data texture.
///
/// 5 x `vec4<u32>` = 80 bytes, matching the WGSL `GlyphInfo` struct exactly.
///
/// ```text
/// texel/slot 0  (u32 x4): bbox_xy, bbox_wh, curves_start, curves_end
///   bbox_xy  = (xmin as u16) | ((ymin as u16) << 16)
///   bbox_wh  = (xmax as u16) | ((ymax as u16) << 16)
/// texel/slot 1  (u32 x4): vband[0..3]
/// texel/slot 2  (u32 x4): vband[4..7]
/// texel/slot 3  (u32 x4): hband[0..3]
/// texel/slot 4  (u32 x4): hband[4..7]
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SlugGlyph {
    pub data: [u32; 4],
    pub vband: [u32; DEFAULT_BAND_COUNT],
    pub hband: [u32; DEFAULT_BAND_COUNT],
}

const _: () = assert!(
    std::mem::size_of::<SlugGlyph>() == 80,
    "SlugGlyph must be exactly 80 bytes"
);

impl SlugGlyph {
    fn from_cached(
        cached: &CachedGlyph,
        curves_start: u32,
        curves_end: u32,
        indices_base: u32,
    ) -> Self {
        let [xmin, ymin, xmax, ymax] = cached.bbox;
        let bbox_xy = (xmin as u16 as u32) | ((ymin as u16 as u32) << 16);
        let bbox_wh = (xmax as u16 as u32) | ((ymax as u16 as u32) << 16);

        let mut vband = [0u32; DEFAULT_BAND_COUNT];
        let mut hband = [0u32; DEFAULT_BAND_COUNT];

        for (i, item) in vband.iter_mut().enumerate().take(DEFAULT_BAND_COUNT) {
            let pr = cached.vband[i];
            *item = PackedRange::new(pr.offset() + indices_base, pr.count()).0;
        }
        for (i, item) in hband.iter_mut().enumerate().take(DEFAULT_BAND_COUNT) {
            let pr = cached.hband[i];
            *item = PackedRange::new(pr.offset() + indices_base, pr.count()).0;
        }

        SlugGlyph {
            data: [bbox_xy, bbox_wh, curves_start, curves_end],
            vband,
            hband,
        }
    }
}

/// One vertex in a slug text mesh.
///
/// Layout (28 bytes, stride 28) matching gabdube's vertex format:
/// ```text
/// offset  0: pos   [f32; 4]  - screen-space xy + corner sign xy (unused but reserved)
/// offset 16: glyph [u32; 2]  - [pack_i16(em_x, em_y), absolute_glyph_index]
/// offset 24: color [u8;  4]  - RGBA8
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SlugVertex {
    /// Screen-space xy. z/w hold the corner sign (±1) for potential future use.
    pub pos: [f32; 4],
    /// `[0]` = `(em_x as i16 as u16) | ((em_y as i16 as u16) << 16)`
    /// `[1]` = absolute glyph index in the combined frame atlas
    pub glyph: [u32; 2],
    /// RGBA8 vertex color.
    pub color: [u8; 4],
}

const _: () = assert!(
    std::mem::size_of::<SlugVertex>() == 28,
    "SlugVertex must be exactly 28 bytes"
);

#[inline(always)]
fn pack_i16(hi: i16, lo: i16) -> u32 {
    ((hi as u16 as u32) << 16) | (lo as u16 as u32)
}

/// One entry in the glyph layout buffer consumed by the UiMaterial fragment
/// shader. The fragment receives only a UV coordinate from Bevy's vertex
/// shader, so the CPU pre-computes the screen-space and em-space extents for
/// every shaped glyph here.
///
/// 48 bytes = 3 x `vec4<f32>`, naturally aligned for std430 / storage buffers.
///
/// ```text
/// screen_rect: [xmin, ymin, xmax, ymax]  - node-space pixels (top-left origin)
/// em_rect:     [xmin, ymax, xmax, ymin]  - raw font units; NOTE y is stored
///              "flipped" so that mix(em_rect.xy, em_rect.zw, t) where t comes
///              from the screen rect gives the correct em coordinate (font y
///              increases upward while screen y increases downward).
/// glyph_index: absolute index into the combined frame atlas glyphs[]
/// _pad:        [u32; 3]
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SlugGlyphLayout {
    pub screen_rect: [f32; 4],
    pub em_rect: [f32; 4],
    pub glyph_index: u32,
    pub _pad: [u32; 3],
}

const _: () = assert!(
    std::mem::size_of::<SlugGlyphLayout>() == 48,
    "SlugGlyphLayout must be exactly 48 bytes"
);

/// A fully-shaped text string: screen-space quads, em-space coords, and
/// absolute glyph indices, ready to upload as a vertex/index buffer.
///
/// Produced by [`SlugAtlas::shape`]. Invalidated whenever the frame atlas is
/// recompacted (call `shape` again after `build_frame_atlas` returns `true`).
///
/// Coordinates in `glyph_layout` are always in natural-origin space:
/// glyphs start at x=0, y=ascender_px, with no layout transform applied.
/// The shader applies scale and offset at draw time via `slugtext()`.
#[derive(Clone, Debug, Default)]
pub struct SlugTextRun {
    pub vertices: Vec<SlugVertex>,
    pub indices: Vec<u32>,
    /// Per-glyph layout for the UiMaterial UV-based fragment shader path.
    /// One entry per rendered glyph (whitespace excluded), same order as the
    /// vertex quads. Coords are natural-origin (no layout applied).
    pub glyph_layout: Vec<SlugGlyphLayout>,
    pub font_id: FontId,
    /// Total horizontal advance of the run at `font_size` in pixels.
    /// Used by the shader layout math to compute em_scale when fitting text
    /// into a rect. Equal to `cursor_x` at the end of `shape()`.
    pub natural_advance: f32,
    /// Cap height of the run at `font_size` in pixels: (ascender − descender) x scale.
    /// Used by the shader to compute the vertical scale factor.
    pub natural_height: f32,
}

impl SlugTextRun {
    /// True if this run produced at least one renderable glyph quad.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Result of [`SlugAtlas::validate_glyphs`].
#[derive(Debug, Default)]
pub struct EnsureGlyphsResult {
    /// Characters for which new CPU-cache entries were created this call.
    pub newly_added: Vec<char>,
    /// Characters the font has no usable outline for (space, tab, unknown, etc).
    pub missing: Vec<char>,
}

impl EnsureGlyphsResult {
    /// True if at least one new glyph was added to the CPU cache.
    pub fn atlas_grew(&self) -> bool {
        !self.newly_added.is_empty()
    }
}

/// Records where a particular font's data starts in the combined frame atlas.
/// Built by `build_frame_atlas` and consumed by `shape`.
#[derive(Clone, Debug, Default)]
struct FontSlot {
    /// Absolute index into `FrameAtlas::glyphs` where this font's glyphs begin.
    glyph_base: u32,
    /// Maps each GlyphId (raw u16) to its absolute frame glyph index.
    glyph_index_map: HashMap<u16, u32>,
}

/// The compacted GPU buffers for the current frame's visible glyphs.
/// Rebuilt by `build_frame_atlas` whenever the visible glyph set changes.
#[derive(Default)]
pub struct FrameAtlas {
    /// Packed `SlugGlyph` structs (80 bytes each) for all fonts combined.
    pub glyphs: Vec<u8>,
    /// Packed `[f32; 6]` curve data (24 bytes per curve, no padding).
    pub curves: Vec<u8>,
    /// Packed `u32` curve index array.
    pub curve_indices: Vec<u8>,
    /// Per-font slot info (for shape() to look up absolute glyph indices).
    slots: Vec<FontSlot>,
    /// Hash of the glyph-id sets last used to build this atlas.
    last_hash: u64,
    /// Set to true whenever the frame atlas was rebuilt this cycle.
    pub dirty: bool,
}

impl FrameAtlas {
    fn clear_buffers(&mut self) {
        self.glyphs.clear();
        self.curves.clear();
        self.curve_indices.clear();
        self.slots.clear();
    }

    /// Returns the absolute glyph index for `(font_id, glyph_id)` if present.
    pub fn glyph_index(&self, font_id: FontId, glyph_id: u16) -> Option<u32> {
        self.slots
            .get(font_id.0 as usize)?
            .glyph_index_map
            .get(&glyph_id)
            .copied()
    }
}

struct FontEntry {
    /// Raw font bytes kept alive for the `Face` borrow.
    _data: Vec<u8>,
    /// Parsed face (borrows `_data` but we use unsafe to extend lifetime).
    face: Face<'static>,
    /// Whether the font uses TrueType or CFF/CFF2 outlines.
    pub outline_type: OutlineType,
    /// Permanent CPU cache: glyph_id.0 -> CachedGlyph.
    glyph_cache: HashMap<u16, CachedGlyph>,
}

/// Combined multi-font CPU atlas and frame-atlas resource.
///
/// Insert this as a Bevy `Resource`. Call `validate_glyphs` before `shape`
/// for any text that might contain new characters. Call
/// `build_frame_atlas` once per frame after all `validate_glyphs` calls to
/// compact the visible glyph set into GPU buffers. Call `shape` to produce
/// per-string vertex/index data.
#[derive(bevy::prelude::Resource, Default)]
pub struct SlugAtlas {
    fonts: Vec<FontEntry>,
    pub frame: FrameAtlas,
}

impl SlugAtlas {
    /// Register a font from raw TTF/OTF bytes.
    ///
    /// Returns a `FontId` that must be passed to every subsequent API call for
    /// this font. Returns an error if `ttf-parser` cannot parse the data.
    pub fn register_font(&mut self, font_data: Vec<u8>) -> Result<FontId, String> {
        // SAFETY: We keep `_data` alive for the same lifetime as `face` inside
        // `FontEntry` and never expose the raw reference outside.
        let face_data: &'static [u8] =
            unsafe { std::slice::from_raw_parts(font_data.as_ptr(), font_data.len()) };
        let face = Face::parse(face_data, 0)
            .map_err(|e| format!("ttf-parser: failed to parse font: {e:?}"))?;
        // Detect outline format by checking which table is present.
        // `tables().glyf` Some for TrueType outlines, None for CFF.
        let outline_type = if face.tables().glyf.is_some() {
            OutlineType::TrueType
        } else {
            OutlineType::Cff
        };
        let id = FontId(self.fonts.len() as u32);
        self.fonts.push(FontEntry {
            _data: font_data,
            face,
            outline_type,
            glyph_cache: HashMap::new(),
        });
        Ok(id)
    }

    /// Ensure all characters in `text` have entries in the CPU glyph cache for
    /// `font_id`.
    ///
    /// This lets a caller know if a font glyphs can or cannot be parsed into
    /// bezier curves.
    ///
    /// For each character:
    /// - Cache hit  -> skip (O(1) lookup, no TTF work).
    /// - Cache miss -> extract curves + build bands -> insert into cache.
    ///   (marks `result.newly_added`)
    /// - No outline -> record in `result.missing` whitespace, unknown, etc...
    ///
    /// After this call, every character in `result.newly_added` is ready for
    /// `shape` and for inclusion in the next `build_frame_atlas` call.
    pub fn validate_glyphs(&mut self, font_id: FontId, text: &str) -> EnsureGlyphsResult {
        let mut result = EnsureGlyphsResult::default();
        let Some(entry) = self.fonts.get_mut(font_id.0 as usize) else {
            bevy::log::warn!("SlugAtlas::validate_glyphs: unknown FontId {:?}", font_id);
            return result;
        };

        for ch in text.chars() {
            if ch.is_whitespace() {
                // Whitespace has no outline; callers handle advance via
                // glyph_hor_advance fallback in shape().
                continue;
            }
            let Some(gid) = entry.face.glyph_index(ch) else {
                result.missing.push(ch);
                continue;
            };
            let key = gid.0;
            if entry.glyph_cache.contains_key(&key) {
                continue; // already cached
            }
            // Cache miss - extract curves and build band index now.
            match extract_cached_glyph(&entry.face, gid) {
                Some(cached) => {
                    entry.glyph_cache.insert(key, cached);
                    result.newly_added.push(ch);
                }
                None => {
                    // The glyph exists in the font but has no usable outline
                    // (can happen for some control glyphs, .notdef, etc).
                    result.missing.push(ch);
                }
            }
        }

        result
    }

    /// Compact the visible glyph set into GPU-ready byte buffers.
    ///
    /// Pass all `SlugTextRun` font-ids and the text strings that will be
    /// rendered this frame so the atlas knows which glyphs to include.
    ///
    /// Returns `true` if the atlas was rebuilt (the visible set changed).
    /// Returns `false` if nothing changed; existing GPU buffers remain valid.
    ///
    /// After a `true` return, all existing [`SlugTextRun`]s are stale (their
    /// absolute glyph indices may no longer be valid). Call `shape` again
    /// for every string before drawing.
    pub fn build_frame_atlas(
        &mut self,
        // Iterator of (font_id, glyph_ids_needed) pairs.
        // Each glyph_id is the raw u16 from `Face::glyph_index`.
        needed: &[(FontId, Vec<u16>)],
    ) -> bool {
        // Compute a cheap hash of the (font_id, glyph_id) sets so we can skip
        // a rebuild when nothing changed.
        let new_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (fid, ids) in needed {
                fid.0.hash(&mut h);
                let mut sorted = ids.clone();
                sorted.sort_unstable();
                sorted.hash(&mut h);
            }
            h.finish()
        };

        if new_hash == self.frame.last_hash {
            self.frame.dirty = false;
            return false;
        }

        // Rebuild from scratch.
        self.frame.clear_buffers();
        self.frame.last_hash = new_hash;

        let mut abs_curves_base: u32 = 0;
        let mut abs_indices_base: u32 = 0;

        // Ensure slots vec has the right length.
        while self.frame.slots.len() < self.fonts.len() {
            self.frame.slots.push(FontSlot::default());
        }
        for slot in &mut self.frame.slots {
            slot.glyph_index_map.clear();
            slot.glyph_base = 0;
        }

        let mut abs_glyph_idx: u32 = 0;

        for (font_id, glyph_ids) in needed {
            let fi = font_id.0 as usize;
            let Some(entry) = self.fonts.get(fi) else {
                continue;
            };

            self.frame.slots[fi].glyph_base = abs_glyph_idx;

            // Deduplicate and sort so frame layout is deterministic.
            let mut unique = glyph_ids.clone();
            unique.sort_unstable();
            unique.dedup();

            for &gid in &unique {
                let Some(cached) = entry.glyph_cache.get(&gid) else {
                    continue; // glyph not in cache (whitespace / missing)
                };

                let curves_start = abs_curves_base;
                for c in &cached.curves {
                    // 24 bytes per curve: p0, p1, p2 each as [f32;2]
                    for &v in c.p0.iter().chain(c.p1.iter()).chain(c.p2.iter()) {
                        self.frame.curves.extend_from_slice(&v.to_le_bytes());
                    }
                }
                let curves_end = abs_curves_base + cached.curves.len() as u32;

                let indices_start = abs_indices_base;
                for &local_idx in &cached.local_curve_indices {
                    let abs_idx = local_idx + curves_start;
                    self.frame
                        .curve_indices
                        .extend_from_slice(&abs_idx.to_le_bytes());
                }
                let _indices_end = abs_indices_base + cached.local_curve_indices.len() as u32;

                let gpu_glyph =
                    SlugGlyph::from_cached(cached, curves_start, curves_end, indices_start);
                self.frame
                    .glyphs
                    .extend_from_slice(bytemuck::bytes_of(&gpu_glyph));

                // Record mapping for shape().
                self.frame.slots[fi]
                    .glyph_index_map
                    .insert(gid, abs_glyph_idx);

                abs_curves_base += cached.curves.len() as u32;
                abs_indices_base += cached.local_curve_indices.len() as u32;
                abs_glyph_idx += 1;
            }
        }

        self.frame.dirty = true;
        true
    }

    /// Shape `text` into natural-origin coordinates at `font_size`.
    ///
    /// Output coordinates always start at `(0, ascender_px)` - no layout
    /// transform is applied. The caller (shader) handles placement and
    /// alignment via `slugtext(px, rect, layout, run_idx)`.
    ///
    /// `color` is RGBA8 and is stored in the vertex mesh for the Material2d
    /// path. For the UiMaterial path, color lives on `SlugMaterial.text_color`
    /// and is never read from the vertices.
    ///
    /// Call after `build_frame_atlas` so absolute glyph indices are fresh.
    /// Returns `None` if `font_id` is invalid.
    pub fn shape(
        &self,
        font_id: FontId,
        text: &str,
        font_size: f32,
        color: [u8; 4],
    ) -> Option<SlugTextRun> {
        let entry = self.fonts.get(font_id.0 as usize)?;
        let slot = self.frame.slots.get(font_id.0 as usize)?;

        let upm = entry.face.units_per_em() as f32;
        let scale = font_size / upm;
        let ascender_px = entry.face.ascender() as f32 * scale;
        let descender_px = entry.face.descender() as f32 * scale; // negative

        let mut vertices: Vec<SlugVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut glyph_layout: Vec<SlugGlyphLayout> = Vec::new();
        // cursor_x always starts at 0 - natural origin, no x/y offset.
        let mut cursor_x: f32 = 0.0;

        for ch in text.chars() {
            let (gid_opt, advance_px) = if let Some(gid) = entry.face.glyph_index(ch) {
                let adv = entry
                    .face
                    .glyph_hor_advance(gid)
                    .map(|v| v as f32 * scale)
                    .unwrap_or(upm * 0.5 * scale);
                (Some(gid), adv)
            } else {
                (None, upm * 0.25 * scale)
            };

            let gid = match gid_opt {
                Some(g) => g,
                None => {
                    cursor_x += advance_px;
                    continue;
                }
            };
            let cached = match entry.glyph_cache.get(&gid.0) {
                Some(c) => c,
                None => {
                    cursor_x += advance_px;
                    continue;
                }
            };
            let abs_glyph_idx = match slot.glyph_index_map.get(&gid.0) {
                Some(&i) => i,
                None => {
                    cursor_x += advance_px;
                    continue;
                }
            };

            let [xmin_em, ymin_em, xmax_em, ymax_em] = cached.bbox.map(|v| v as f32);
            let xmin_px = cursor_x + xmin_em * scale;
            let xmax_px = cursor_x + xmax_em * scale;

            // Natural origin: y=0 is the top of the node; ascender sits at y=0.
            // Font y increases upward, screen y increases downward - flip via ascender.
            let ymin_px = ascender_px - ymax_em * scale;
            let ymax_px = ascender_px - ymin_em * scale;

            let base = vertices.len() as u32;
            indices.extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);

            let [xmin_i, ymin_i, xmax_i, ymax_i] = cached.bbox;
            vertices.push(SlugVertex {
                pos: [xmin_px, ymin_px, -1.0, 1.0],
                glyph: [pack_i16(xmin_i, ymax_i), abs_glyph_idx],
                color,
            });
            vertices.push(SlugVertex {
                pos: [xmax_px, ymin_px, 1.0, 1.0],
                glyph: [pack_i16(xmax_i, ymax_i), abs_glyph_idx],
                color,
            });
            vertices.push(SlugVertex {
                pos: [xmin_px, ymax_px, -1.0, -1.0],
                glyph: [pack_i16(xmin_i, ymin_i), abs_glyph_idx],
                color,
            });
            vertices.push(SlugVertex {
                pos: [xmax_px, ymax_px, 1.0, -1.0],
                glyph: [pack_i16(xmax_i, ymin_i), abs_glyph_idx],
                color,
            });

            // UiMaterial layout entry - natural-origin screen_rect, y-flipped em_rect.
            glyph_layout.push(SlugGlyphLayout {
                screen_rect: [xmin_px, ymin_px, xmax_px, ymax_px],
                em_rect: [xmin_i as f32, ymax_i as f32, xmax_i as f32, ymin_i as f32],
                glyph_index: abs_glyph_idx,
                _pad: [0; 3],
            });

            cursor_x += advance_px;
        }

        // natural_advance = total x advance; natural_height = ascender - descender
        // (descender is negative, so subtracting it adds to height).
        let natural_advance = cursor_x;
        let natural_height = ascender_px - descender_px;

        Some(SlugTextRun {
            vertices,
            indices,
            glyph_layout,
            font_id,
            natural_advance,
            natural_height,
        })
    }

    /// How many fonts are registered.
    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    /// How many glyphs are cached for the given font.
    pub fn cached_glyph_count(&self, font_id: FontId) -> usize {
        self.fonts
            .get(font_id.0 as usize)
            .map(|e| e.glyph_cache.len())
            .unwrap_or(0)
    }

    /// Returns underlying font `(ascender, descender)` in raw font units for a `font_id`.
    /// These are the values from [`ttf_parser::Face::ascender`] and
    /// [`ttf_parser::Face::descender`] descender is typically negative
    /// generally for most fonts. I need more unit tests.
    /// Returns the outline type TrueType or CFF for a font.
    ///
    /// Needed by `build_text_3d_mesh` to pick the correct contour winding
    /// interpretation when classifying outer contours versus holes.
    pub fn font_outline_type(&self, font_id: FontId) -> Option<OutlineType> {
        self.fonts.get(font_id.0 as usize).map(|e| e.outline_type)
    }

    pub fn font_metrics(&self, font_id: FontId) -> Option<(f32, f32)> {
        let entry = self.fonts.get(font_id.0 as usize)?;
        Some((entry.face.ascender() as f32, entry.face.descender() as f32))
    }

    // TODO: Result for this if the font or glyphs not cached? None is a hack
    // for now.
    /// Return a reference to the cached glyph data for `glyph_id` under
    /// `font_id`, or `None` if the font or glyph is not in the cache.
    pub fn cached_glyph(&self, font_id: FontId, glyph_id: u16) -> Option<&CachedGlyph> {
        self.fonts
            .get(font_id.0 as usize)?
            .glyph_cache
            .get(&glyph_id)
    }

    /// Return `units_per_em` for `font_id`.
    pub fn units_per_em(&self, font_id: FontId) -> Option<u16> {
        Some(self.fonts.get(font_id.0 as usize)?.face.units_per_em())
    }

    // TODO: Here too might be a better option for a Result rather than Option
    // as I'm mixing concerns a bit for simplicity/v0.
    /// Look up the ttf/font glyph id as u16 for some unicode character in
    /// `font_id`.
    ///
    /// Returns `None` if the font is not registered or the character has no
    /// glyph in the font.
    pub fn glyph_index_for_char(&self, font_id: FontId, ch: char) -> Option<u16> {
        self.fonts
            .get(font_id.0 as usize)?
            .face
            .glyph_index(ch)
            .map(|g| g.0)
    }

    /// Return horizontal advance for `glyph_id` in font units. Returns `None`
    /// if the font is not registered or the glyph has no advance metric.
    ///
    /// Callers that need pixel-scaled advance should multiply by
    /// `font_size / units_per_em`.
    pub fn glyph_advance(&self, font_id: FontId, glyph_id: u16) -> Option<f32> {
        self.fonts
            .get(font_id.0 as usize)?
            .face
            .glyph_hor_advance(ttf_parser::GlyphId(glyph_id))
            .map(|v| v as f32)
    }

    /// Collect all glyph ids needed to render `text` with `font_id`.
    /// Returns an empty vec if font_id is invalid or the character has no glyph.
    pub fn collect_glyph_ids(&self, font_id: FontId, text: &str) -> Vec<u16> {
        let Some(entry) = self.fonts.get(font_id.0 as usize) else {
            return vec![];
        };
        let mut ids: Vec<u16> = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .filter_map(|c| entry.face.glyph_index(c).map(|g| g.0))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// Extract curves and build band index for one glyph.
/// Returns `None` if the glyph has no usable outline (space, tab, .notdef, etc).
fn extract_cached_glyph(face: &Face<'_>, gid: GlyphId) -> Option<CachedGlyph> {
    let mut builder = CurveBuilder::default();
    let rect = face.outline_glyph(gid, &mut builder)?;
    let (curves, contour_starts) = builder.into_data();
    if curves.is_empty() {
        return None;
    }

    // Zero-area glyphs produce degenerate bands - skip them.
    let w = (rect.x_max - rect.x_min) as f32;
    let h = (rect.y_max - rect.y_min) as f32;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    let bbox = [rect.x_min, rect.y_min, rect.x_max, rect.y_max];
    let (local_curve_indices, vband, hband) = build_band_index(&curves, &rect, DEFAULT_BAND_COUNT);

    Some(CachedGlyph {
        bbox,
        curves,
        contour_starts,
        local_curve_indices,
        vband,
        hband,
    })
}

/// Build the spatial band index for a glyph.
///
/// Returns `(local_curve_indices, vband, hband)`.
/// All offsets are local (0-based within the returned `local_curve_indices`).
fn build_band_index(
    curves: &[QuadCurve],
    bbox: &Rect,
    band_count: usize,
) -> (
    Vec<u32>,
    [PackedRange; DEFAULT_BAND_COUNT],
    [PackedRange; DEFAULT_BAND_COUNT],
) {
    let xmin = bbox.x_min as f32;
    let ymin = bbox.y_min as f32;
    let w = (bbox.x_max - bbox.x_min) as f32;
    let h = (bbox.y_max - bbox.y_min) as f32;
    let bc = band_count as f32;

    // Accumulate which curves fall in each band.
    let mut h_lists: Vec<Vec<u32>> = vec![Vec::new(); band_count];
    let mut v_lists: Vec<Vec<u32>> = vec![Vec::new(); band_count];

    for (ci, curve) in curves.iter().enumerate() {
        let [cxmin, cymin, cxmax, cymax] = curve.bbox();

        if h > 0.0 {
            let b0 = (((cymin - ymin) / h * bc).floor() as isize).clamp(0, band_count as isize - 1)
                as usize;
            let b1 = (((cymax - ymin) / h * bc).floor() as isize).clamp(0, band_count as isize - 1)
                as usize;
            for item in h_lists[b0..=b1].iter_mut() {
                item.push(ci as u32);
            }
        }
        if w > 0.0 {
            let b0 = (((cxmin - xmin) / w * bc).floor() as isize).clamp(0, band_count as isize - 1)
                as usize;
            let b1 = (((cxmax - xmin) / w * bc).floor() as isize).clamp(0, band_count as isize - 1)
                as usize;
            for item in v_lists[b0..=b1].iter_mut() {
                item.push(ci as u32);
            }
        }
    }

    // Sort h-bands descending by curve max-x (early exit in shader).
    for list in &mut h_lists {
        list.sort_unstable_by(|&a, &b| {
            curves[b as usize]
                .max_x()
                .partial_cmp(&curves[a as usize].max_x())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    // Sort v-bands descending by curve max-y.
    for list in &mut v_lists {
        list.sort_unstable_by(|&a, &b| {
            curves[b as usize]
                .max_y()
                .partial_cmp(&curves[a as usize].max_y())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Pack everything into a single flat `local_curve_indices` array and build
    // packed range descriptors.
    let mut local_curve_indices: Vec<u32> = Vec::new();
    let mut vband = [PackedRange::default(); DEFAULT_BAND_COUNT];
    let mut hband = [PackedRange::default(); DEFAULT_BAND_COUNT];

    // h-bands first.
    for (i, list) in h_lists.iter().enumerate() {
        let offset = local_curve_indices.len() as u32;
        let count = list.len() as u32;
        hband[i] = PackedRange::new(offset, count);
        local_curve_indices.extend_from_slice(list);
    }
    // v-bands after h-bands in the same flat array.
    for (i, list) in v_lists.iter().enumerate() {
        let offset = local_curve_indices.len() as u32;
        let count = list.len() as u32;
        vband[i] = PackedRange::new(offset, count);
        local_curve_indices.extend_from_slice(list);
    }

    (local_curve_indices, vband, hband)
}

#[derive(Default)]
struct CurveBuilder {
    curves: Vec<QuadCurve>,
    cur: [f32; 2],
    start: [f32; 2],
    contour_starts: Vec<usize>,
}

impl CurveBuilder {
    fn push(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) {
        self.curves.push(QuadCurve { p0, p1, p2 });
    }
    // TODO: Instead of a vec tuple probably make a Struct for this too.
    /// Consume the builder so far and return `(curves, contour_starts)`.
    fn into_data(self) -> (Vec<QuadCurve>, Vec<usize>) {
        (self.curves, self.contour_starts)
    }
}

impl OutlineBuilder for CurveBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contour_starts.push(self.curves.len());
        self.cur = [x, y];
        self.start = [x, y];
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(c) = line_as_quad(self.cur, [x, y]) {
            self.push(c.p0, c.p1, c.p2);
        }
        self.cur = [x, y];
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push(self.cur, [x1, y1], [x, y]);
        self.cur = [x, y];
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let [cx, cy] = self.cur;
        let m01 = lerp2([cx, cy], [x1, y1], 0.5);
        let m12 = lerp2([x1, y1], [x2, y2], 0.5);
        let m23 = lerp2([x2, y2], [x, y], 0.5);
        let m012 = lerp2(m01, m12, 0.5);
        let m123 = lerp2(m12, m23, 0.5);
        let mid = lerp2(m012, m123, 0.5);
        self.push(self.cur, m01, mid);
        self.push(mid, m123, [x, y]);
        self.cur = [x, y];
    }
    fn close(&mut self) {
        let [sx, sy] = self.start;
        let [cx, cy] = self.cur;
        if ((sx - cx).abs() > 0.1 || (sy - cy).abs() > 0.1)
            && let Some(c) = line_as_quad(self.cur, self.start)
        {
            self.push(c.p0, c.p1, c.p2);
        }
        self.cur = self.start;
    }
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn line_as_quad(p0: [f32; 2], p2: [f32; 2]) -> Option<QuadCurve> {
    let dx = p2[0] - p0[0];
    let dy = p2[1] - p0[1];
    if dx.abs() < 0.1 && dy.abs() < 0.1 {
        return None;
    }
    let mx = (p0[0] + p2[0]) * 0.5;
    let my = (p0[1] + p2[1]) * 0.5;
    let len = (dx * dx + dy * dy).sqrt();
    let (p1x, p1y) = if len > 0.0 {
        let inv = LINE_EPSILON / len;
        (mx - dy * inv, my + dx * inv)
    } else {
        (mx, my)
    };
    Some(QuadCurve {
        p0,
        p1: [p1x, p1y],
        p2,
    })
}

// TODO: Rip out the wgpu testing harness and just use bevy apps

// The headless render test harness (`render.rs` + `tests/shader_slug.rs`) still
// uses the old flat-buffer API. These thin wrappers bridge the gap so the
// tests continue to compile and pass without modification until they are
// updated to the new vertex-based approach.

/// Old-API compat: build a flat atlas for a single font + text string.
/// Used only by the headless render test harness.
pub fn build_slug_atlas(
    font_data: &[u8],
    text: &str,
    band_count: usize,
) -> Result<LegacySlugAtlas, String> {
    let _ = band_count; // kept for API compat, always uses DEFAULT_BAND_COUNT
    let mut atlas = SlugAtlas::default();
    let fid = atlas.register_font(font_data.to_vec())?;
    atlas.validate_glyphs(fid, text);

    let ids = atlas.collect_glyph_ids(fid, text);
    atlas.build_frame_atlas(&[(fid, ids)]);

    // Extract old-style flat buffers from the new frame atlas.
    // The headless tests only care that the buffers are non-empty and
    // have the right relative sizes; exact byte layout differences are
    // acceptable for this compat shim.
    let glyphs_count = atlas.frame.glyphs.len() / std::mem::size_of::<SlugGlyph>();

    Ok(LegacySlugAtlas {
        glyphs_bytes: atlas.frame.glyphs.clone(),
        curves_bytes: atlas.frame.curves.clone(),
        curve_indices_bytes: atlas.frame.curve_indices.clone(),
        glyph_count: glyphs_count as u32,
    })
}

/// Flat byte buffers compatible with the old render-test API.
pub struct LegacySlugAtlas {
    pub glyphs_bytes: Vec<u8>,
    pub curves_bytes: Vec<u8>,
    pub curve_indices_bytes: Vec<u8>,
    pub glyph_count: u32,
}

/// Legacy font validation - unchanged API surface.
pub fn check_font_chars(font_data: &[u8], chars: &str) -> Result<(), Vec<char>> {
    let face = match Face::parse(font_data, 0) {
        Ok(f) => f,
        Err(_) => {
            let bad = chars.chars().filter(|c| !c.is_whitespace()).collect();
            return Err(bad);
        }
    };
    struct Sink;
    impl OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let mut failures: Vec<char> = Vec::new();
    for ch in chars.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let Some(gid) = face.glyph_index(ch) else {
            failures.push(ch);
            continue;
        };
        if face.outline_glyph(gid, &mut Sink).is_none() {
            failures.push(ch);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Legacy ASCII font check - unchanged API surface.
pub fn check_font_ascii(font_data: &[u8]) -> Result<(), Vec<char>> {
    let printable: String = (0x21u8..=0x7Eu8)
        .map(|b| b as char)
        .chain([' ', '\t', '\n'])
        .collect();
    check_font_chars(font_data, &printable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRA_TTF: &[u8] = include_bytes!("../tests/fixtures/FiraMono-Medium.ttf");
    const INTER_TTF: &[u8] = include_bytes!("../tests/fixtures/Inter.ttf");

    fn make_atlas(font_data: &[u8]) -> (SlugAtlas, FontId) {
        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(font_data.to_vec())
            .expect("register_font");
        (atlas, fid)
    }

    #[test]
    fn register_font_returns_sequential_ids() {
        let mut atlas = SlugAtlas::default();
        let a = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font failed");
        let b = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font failed");
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
    }

    #[test]
    fn register_invalid_bytes_returns_error() {
        let mut atlas = SlugAtlas::default();
        assert!(atlas.register_font(vec![0u8; 16]).is_err());
    }

    #[test]
    fn validate_glyphs_warms_cache_for_printable() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        let r = atlas.validate_glyphs(fid, "Hi");
        assert!(!r.newly_added.is_empty(), "H and i must be newly added");
        assert!(r.missing.is_empty(), "no missing for known printable chars");
        assert_eq!(atlas.cached_glyph_count(fid), 2);
    }

    #[test]
    fn validate_glyphs_no_duplicate_work() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        let r1 = atlas.validate_glyphs(fid, "aaa");
        assert_eq!(r1.newly_added.len(), 1, "first call: only 'a' is new");
        let r2 = atlas.validate_glyphs(fid, "aaa");
        assert!(r2.newly_added.is_empty(), "second call: 'a' already cached");
        assert_eq!(atlas.cached_glyph_count(fid), 1);
    }

    #[test]
    fn validate_glyphs_skips_whitespace() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        let r = atlas.validate_glyphs(fid, " \t\n");
        assert!(r.newly_added.is_empty());
        assert!(r.missing.is_empty(), "whitespace is not 'missing'");
    }

    #[test]
    fn validate_glyphs_reports_inter_as_missing() {
        let (mut atlas, fid) = make_atlas(INTER_TTF);
        let r = atlas.validate_glyphs(fid, "Hi");
        // Inter.ttf gvar variant: outline_glyph returns None.
        assert!(!r.missing.is_empty(), "Inter.ttf glyphs must be missing");
    }

    #[test]
    fn build_frame_atlas_produces_nonempty_buffers() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "Hello world!");
        let ids = atlas.collect_glyph_ids(fid, "Hello world!");
        let rebuilt = atlas.build_frame_atlas(&[(fid, ids)]);
        assert!(rebuilt, "first build must return true");
        assert!(!atlas.frame.glyphs.is_empty());
        assert!(!atlas.frame.curves.is_empty());
        assert!(!atlas.frame.curve_indices.is_empty());
    }

    #[test]
    fn build_frame_atlas_stable_hash_returns_false() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "abc");
        let ids = atlas.collect_glyph_ids(fid, "abc");
        atlas.build_frame_atlas(&[(fid, ids.clone())]);
        let rebuilt = atlas.build_frame_atlas(&[(fid, ids)]);
        assert!(!rebuilt, "same glyph set must not trigger rebuild");
    }

    #[test]
    fn build_frame_atlas_glyph_size_multiple_of_80() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "Hi");
        let ids = atlas.collect_glyph_ids(fid, "Hi");
        atlas.build_frame_atlas(&[(fid, ids)]);
        assert_eq!(
            atlas.frame.glyphs.len() % std::mem::size_of::<SlugGlyph>(),
            0,
            "glyph buffer must be a multiple of SlugGlyph size (80 bytes)"
        );
    }

    #[test]
    fn build_frame_atlas_curve_bytes_multiple_of_24() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "Hi");
        let ids = atlas.collect_glyph_ids(fid, "Hi");
        atlas.build_frame_atlas(&[(fid, ids)]);
        assert_eq!(
            atlas.frame.curves.len() % 24,
            0,
            "curve buffer must be a multiple of 24 bytes (6 x f32)"
        );
    }

    #[test]
    fn build_frame_atlas_deduplicates_glyphs() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "aaa");
        let ids = atlas.collect_glyph_ids(fid, "aaa");
        atlas.build_frame_atlas(&[(fid, ids)]);
        let glyph_count = atlas.frame.glyphs.len() / std::mem::size_of::<SlugGlyph>();
        assert_eq!(glyph_count, 1, "'aaa' must produce exactly one glyph entry");
    }

    #[test]
    fn shape_produces_correct_vertex_index_counts() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "Hi");
        let ids = atlas.collect_glyph_ids(fid, "Hi");
        atlas.build_frame_atlas(&[(fid, ids)]);

        let run = atlas
            .shape(fid, "Hi", 32.0, [255; 4])
            .expect("shape must return Some for valid font");
        // 2 glyphs x 4 verts = 8 vertices
        assert_eq!(run.vertices.len(), 8);
        // 2 glyphs x 6 indices = 12 indices
        assert_eq!(run.indices.len(), 12);
    }

    #[test]
    fn shape_skips_whitespace_quads() {
        let (mut atlas, fid) = make_atlas(FIRA_TTF);
        atlas.validate_glyphs(fid, "a b");
        let ids = atlas.collect_glyph_ids(fid, "a b");
        atlas.build_frame_atlas(&[(fid, ids)]);

        let run = atlas.shape(fid, "a b", 32.0, [255; 4]).expect("shape");
        // 'a' and 'b' get quads; space is skipped
        assert_eq!(run.vertices.len(), 8, "only 'a' and 'b' produce quads");
    }

    #[test]
    fn shape_returns_none_for_invalid_font_id() {
        let atlas = SlugAtlas::default();
        assert!(atlas.shape(FontId(99), "hi", 32.0, [255; 4]).is_none());
    }

    #[test]
    fn check_font_ascii_passes_for_fira() {
        assert!(check_font_ascii(FIRA_TTF).is_ok());
    }

    #[test]
    fn check_font_ascii_fails_for_inter() {
        assert!(check_font_ascii(INTER_TTF).is_err());
    }

    #[test]
    fn check_font_chars_skips_whitespace() {
        assert!(check_font_chars(INTER_TTF, " \t\n").is_ok());
    }
}
