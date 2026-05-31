// Slug glyph extrusion aka just a side wall quad strip mesh builder.
//
// Converts flan's per glyph bezier contours 3d side-wall geometry as
// simple/stupid quad strips.
//
// Outward normals use the right-perpendicular of each world-space edge:
//   normal = (dy/|e|, −dx/|e|, 0)
// This is correct for both CCW outer contours and CW inner contours that would
// be holes in say lyon without any explicit inside/outside classification,
// because TrueType winding conventions guarantee that inner contour edges run
// in the opposite direction to outer contour edges.
//
// Triangle winding is `[v0,v2,v1, v0,v3,v2]` which produces outward-facing
// normals under Bevy's default CCW front-face convention. Setting
// `double_sided: true` on the `StandardMaterial` is here more to handle camera
// angles that peek at the inner surfaces. But probably for my use cases I
// should remove it.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;

use crate::slug::{CachedGlyph, FontId, SlugAtlas};

/// Evaluate `subdivision` of uniformly-spaced points along the quadratic bezier
/// `p0->p1->p2` at `t = 0/n, 1/n, ..., (n-1)/n`. Note endpoint `t=1` is omitted
/// so adjacent curves do not share duplicate vertexes.
pub fn linearize_quad(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    subdivision: usize,
) -> impl Iterator<Item = [f32; 2]> {
    let n = subdivision.max(1);
    (0..n).map(move |i| {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        [
            mt * mt * p0[0] + 2.0 * mt * t * p1[0] + t * t * p2[0],
            mt * mt * p0[1] + 2.0 * mt * t * p1[1] + t * t * p2[1],
        ]
    })
}

/// Append side-wall geometry for one glyph into the provided vertex/index
/// accumulator.
///
/// # Parameters
/// - `cached`: the glyph's cpu side cache entry.
/// - `cursor_x_em`: horizontal advance origin for this glyph in raw font units.
/// - `ascender`, `descender`: font metrics in raw font units.
/// - `half_adv`: horizontal half-advance in normalized world units. Used for
///   centering in 3d space, its the value from in `normalize_run_3d` basically.
/// - `half_depth`: half the extrusion depth in world units.
/// - `subdivision`: number of line segments per quadratic Bézier (1 = straight
///   lines, 6–8 gives smooth curves without excessive vertex counts).
#[allow(clippy::too_many_arguments)]
pub fn build_glyph_side_walls(
    cached: &CachedGlyph,
    cursor_x_em: f32,
    ascender: f32,
    descender: f32,
    half_adv: f32,
    half_depth: f32,
    subdivision: u8,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) {
    let inv_range = 1.0 / (ascender - descender);
    let sub = subdivision.max(1) as usize;

    for contour in cached.contours() {
        let mut pts: Vec<[f32; 2]> = Vec::new();
        for curve in contour {
            for [ex, ey] in linearize_quad(curve.p0, curve.p1, curve.p2, sub) {
                let wx = (ex + cursor_x_em) * inv_range - half_adv;
                let wy = (ey - ascender) * inv_range + 0.5;
                pts.push([wx, wy]);
            }
        }

        let n = pts.len();
        if n < 2 {
            continue;
        }

        for i in 0..n {
            let [x0, y0] = pts[i];
            let [x1, y1] = pts[(i + 1) % n];

            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-8 {
                continue;
            }

            // Right-perpendicular of the world-space edge direction.
            // Outward for CCW outer contours; still outward for CW inner
            // contours because their edges run in the opposite direction.
            let nx = dy / len;
            let ny = -dx / len;

            let base = positions.len() as u32;

            // v0 = front edge-start, v1 = front edge-end
            // v2 = back  edge-end,   v3 = back  edge-start
            positions.push([x0, y0, half_depth]);
            positions.push([x1, y1, half_depth]);
            positions.push([x1, y1, -half_depth]);
            positions.push([x0, y0, -half_depth]);

            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);

            // CCW winding viewed from outside as viewed by normal direction.
            // Cross-product of: (v2-v0) x (v1-v0) = depth*(dy,-dx,0) ∝ outward normal
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
}

/// Build a combined side-wall `Mesh` for all renderable glyphs in `text`
/// laid out at the same natural origin as `SlugAtlas::shape`.
///
/// The mesh uses `Mesh::ATTRIBUTE_POSITION` and `Mesh::ATTRIBUTE_NORMAL` and is
/// suitable for use as a regular bevy `StandardMaterial`. Set `double_sided:
/// true` on the material to ensure inner glyph walls aka the insides of loops
/// in 'O', 'B', 'R', 'P' etc. are also rendered correctly regardless of camera
/// angle aka inside/out of the shape.
///
/// # Parameters
/// - `atlas`: the flan slug atlas which must contain cached glyphs for all the side wall chars obvs.
/// - `font_id`: which registered font to use.
/// - `text`: the string to extrude.
/// - `half_adv`: world-space horizontal half-advance used for centering
/// - `depth`: total extrusion depth in world units; side walls span +/- depth/2.
/// - `subdivision`: line segments per bezier curve.
pub fn build_text_side_walls(
    atlas: &SlugAtlas,
    font_id: FontId,
    text: &str,
    half_adv: f32,
    depth: f32,
    subdivision: u8,
) -> Mesh {
    let half_depth = depth * 0.5;

    let Some((ascender, descender)) = atlas.font_metrics(font_id) else {
        return empty_mesh();
    };
    let Some(upm) = atlas.units_per_em(font_id) else {
        return empty_mesh();
    };
    let upm = upm as f32;

    // Reuse the same advance accumulation as SlugAtlas::shape so cursor
    // positions match the Slug face geometry precisely.
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Work in raw font units in em space so that cursor_x is em units too.
    // shape() advances cursor_x in pixel space but we need em units here.
    // Since we normalize by (ascender - descender), pixel scale cancels out:
    //   cursor_x_em_norm = cursor_x_px * inv_h = cursor_x_em / (ascender - descender)
    // So accumulate cursor_x_em and apply to inv_range directly.
    let mut cursor_x_em: f32 = 0.0;

    for ch in text.chars() {
        if ch.is_whitespace() {
            // Advance for whitespace which has no glyph geometry to speak of.
            // Use the same fallback as shape(): 0.5 * upm for unknown.
            if let Some(gid) = atlas.glyph_index_for_char(font_id, ch) {
                if let Some(adv) = atlas.glyph_advance(font_id, gid) {
                    cursor_x_em += adv;
                } else {
                    cursor_x_em += upm * 0.5;
                }
            } else {
                // TODO: Less magic values, more const...
                cursor_x_em += upm * 0.5;
            }
            continue;
        }

        let Some(gid) = atlas.glyph_index_for_char(font_id, ch) else {
            cursor_x_em += upm * 0.5;
            continue;
        };
        let Some(cached) = atlas.cached_glyph(font_id, gid) else {
            if let Some(adv) = atlas.glyph_advance(font_id, gid) {
                cursor_x_em += adv;
            }
            continue;
        };

        build_glyph_side_walls(
            cached,
            cursor_x_em,
            ascender,
            descender,
            half_adv,
            half_depth,
            subdivision,
            &mut positions,
            &mut normals,
            &mut indices,
        );

        let adv = atlas.glyph_advance(font_id, gid).unwrap_or(upm * 0.5);
        cursor_x_em += adv;
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn empty_mesh() -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    m.insert_indices(Indices::U32(Vec::new()));
    m
}
