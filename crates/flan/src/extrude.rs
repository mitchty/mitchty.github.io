// Slug glyph extrusion to abuse earcutr to cap off slug text 3d wall extrusions
// front/backs.
//
// TODO: Even this approach has seams if zoomed in. Future me fix it but its
// hard to tell unless you zoom in to the thing like you're functionally blind
// and zoom in almost to the max. Other bug is I need to look at how I'm
// calculating where this goes so that toggling betwixt the 2d decal option and
// extrusion doesn't shift or at least is "good enough for government work" on
// not being obvious.
//
// `build_text_3d_mesh` is the main/only sane entry point. It produces a single
// bevy Mesh containing front/back cap, and side-wall geometry extrusion
// geometry for all glyphs in a string atlas.
//
// Converts flan's per glyph bezier contour 3d side-wall geometry as
// simple/stupid quad strips initially, and then caps them off.
//
// Cap triangulation uses earcutr. Contour winding is
// derived on the fly via the shoelace signed-area formula:
//   positive area CCW in font y up space = outer contour
//   negative area  CW in font y up space = hole
//
// Outward normals use right-perpendicular of each world space Mesh edge where:
//  normal = (dy/|e|, −dx/|e|, 0)
//
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;

use crate::slug::{CachedGlyph, FontId, OutlineType, SlugAtlas};

/// Adaptive subdivision count for a single quadratic bezier segment.
///
/// Computes the rough maximum deviation of the curve from its chord using the
/// closed-form formula for a quadratic bezier to add more segments when a curve
/// is "sharper" than "shallower":
///
/// ```text
///   max_dev = |2·P1 − P0 − P2| / 4
/// ```
///
/// The number of uniform segments needed to keep the per-segment deviation
/// below `tolerance` is `ceil(sqrt(max_dev / tolerance))`. This seems to
/// produce nice curves.
///
/// Returns at least 1 and at most 64 segments so we don't go too crazy on the high end.
pub fn adaptive_subdivisions(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], tolerance: f32) -> usize {
    let dx = 2.0 * p1[0] - p0[0] - p2[0];
    let dy = 2.0 * p1[1] - p0[1] - p2[1];

    // max deviation of the bezier from its chord at t = 0.5
    let flatness = (dx * dx + dy * dy).sqrt() * 0.25;
    if flatness <= tolerance {
        1
    } else {
        ((flatness / tolerance).sqrt().ceil() as usize).clamp(1, 64)
    }
}

/// Evaluate `subdivision` of uniformly-spaced points along the quadratic bezier
/// `p0->p1->p2` at `t = 0/n, 1/n, ..., (n-1)/n`. Note endpoint `t=1` is omitted
/// so adjacent curves do not share duplicate vertexes.
// TODO: I think this is where I went wrong trying to avoid duplicate vertexes
// future me fix it.
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
// TODO ^^^ turns out didn't fix it AGAIN. Future mitch figure out a better
// capping algorithm this ones ass at high zoom and there is still a gap betwixt
// the side walls and the caps.
#[allow(clippy::too_many_arguments)]
pub fn build_glyph_side_walls(
    cached: &CachedGlyph,
    cursor_x_em: f32,
    ascender: f32,
    descender: f32,
    half_adv: f32,
    half_depth: f32,
    tolerance: f32,
    xy_outset: f32,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) {
    // TODO: This might be the source of the very small gap betwixt the cap and
    // the side wall. Future me maybe also see if there is a way to use earcutr
    // to do the sidewall geometry. I only learnt enough to be evil not right.
    let half_depth = half_depth * (1.0 - 0.005);
    let inv_range = 1.0 / (ascender - descender);

    for contour in cached.contours() {
        let mut pts: Vec<[f32; 2]> = Vec::new();
        for curve in contour {
            // Adaptive subdivision so that tight curves get more segments, flat
            // sections get as few as 1 for a flat segment. Positions here are in
            // em-space so the flatness is in the same coordinate system as
            // inv_range not in world space like a mesh normally has.
            // TODO: Review thought, maybe I should calculate in world space instead?
            let sub = adaptive_subdivisions(curve.p0, curve.p1, curve.p2, tolerance);
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

            // Right-perpendicular of world-space edge direction.
            // Outward for CCW and CW contours as their edges run in the
            // opposite direction.
            let nx = dy / len;
            let ny = -dx / len;

            let base = positions.len() as u32;

            // Apply XY outset along the outward normal so the side wall extends
            // slightly past the cap coverage boundary. This was SUPPOSED to
            // close the gap with the cap but I'm a dumdum apparently and might
            // be hitting IEE754 issues with mantissa. Future me can figure out
            // where I effed up. I should probably just make a chungus function
            // to do both side walls and caps and be done with it. Some of this
            // is remnants of using the SlugText as a cap and then filling with
            // sidewalls. That approach also failed in the same way just wayyyyy
            // worse.
            let ox0 = x0 + nx * xy_outset;
            let oy0 = y0 + ny * xy_outset;
            let ox1 = x1 + nx * xy_outset;
            let oy1 = y1 + ny * xy_outset;

            // v0 = front edge-start
            // v1 = front edge-end
            // v2 = back edge-end
            // v3 = back edge-start
            positions.push([ox0, oy0, half_depth]);
            positions.push([ox1, oy1, half_depth]);
            positions.push([ox1, oy1, -half_depth]);
            positions.push([ox0, oy0, -half_depth]);

            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);
            normals.push([nx, ny, 0.0]);

            // CCW winding as viewed from outside for normal direction.
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
}

/// Build a combined side-wall `Mesh` for all renderable glyphs in `text`
/// laid out at the same natural origin as `SlugAtlas::shape`.
///
/// Future work should be to have all the meshes be at Origin so that Mesh
/// generation is done once and we just shift/duplicate the mesh in world space
/// as needed.
///
/// The mesh uses `Mesh::ATTRIBUTE_POSITION` and `Mesh::ATTRIBUTE_NORMAL` and is
/// suitable for use as a regular bevy `StandardMaterial`. Set `double_sided:
/// true` on the material to ensure inner glyph walls aka the insides of loops
/// in 'O', 'B', 'R', 'P' etc. are also rendered correctly regardless of camera
/// angle aka inside/out of the shape.
///
/// Not sure if I care about double_sided true anymore, it was an attempt at
/// getting the double decal version of this to work without edge seams. And it
/// failed.
pub fn build_text_side_walls(
    atlas: &SlugAtlas,
    font_id: FontId,
    text: &str,
    half_adv: f32,
    depth: f32,
    tolerance: f32,
    xy_outset: f32,
) -> Mesh {
    let half_depth = depth * 0.5;

    let Some((ascender, descender)) = atlas.font_metrics(font_id) else {
        return empty_mesh();
    };
    let Some(upm) = atlas.units_per_em(font_id) else {
        return empty_mesh();
    };
    let upm = upm as f32;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Work in raw font units in em space so cursor_x is in em units. shape()
    // advances cursor_x in pixel space but we need em units for this
    // calculation.
    // Since we normalize by (ascender - descender), pixel scale cancels out as
    // cursor_x_em_norm = cursor_x_px * inv_h = cursor_x_em / (ascender -
    // descender)
    // Accumulate cursor_x_em and apply to inv_range directly.
    let mut cursor_x_em: f32 = 0.0;

    for ch in text.chars() {
        if ch.is_whitespace() {
            // Advance for whitespace which has no glyph geometry to speak of.
            // Use the same fallback as shape(): 0.5 * upm for unknown.
            // TODO: Future work with Justify will need to have this get wider
            // somehow.
            if let Some(gid) = atlas.glyph_index_for_char(font_id, ch) {
                if let Some(adv) = atlas.glyph_advance(font_id, gid) {
                    cursor_x_em += adv;
                } else {
                    cursor_x_em += upm * 0.5;
                }
            } else {
                // TODO: Less magic numbres, more const...
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
            tolerance,
            xy_outset,
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

/// Ray-casting point-in-polygon test for a linearized polygon.
///
/// Returns `true` if `point` is inside `polygon` winding as CW or CCW, the
/// winding does not affect the result here. Used to assign each hole contour to
/// the correct outer facing contour when a glyph has multiple outer contours.
/// Like most Kanji... cough.
fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let [px, py] = point;
    let n = polygon.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let [xi, yi] = polygon[i];
        let [xj, yj] = polygon[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Compute the signed area of a linearized polygon in font-space via the
/// shoelace formula. Positive = CCW where the outer contour is in TrueType y-up
/// space, negative = CW which is a hole. OTF is the inverse but by the time
/// we're here thats already covered.
fn signed_area(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0_f32;
    for i in 0..n {
        let [x0, y0] = pts[i];
        let [x1, y1] = pts[(i + 1) % n];
        area += x0 * y1 - x1 * y0;
    }
    area * 0.5
}

/// Build a combined front and back cap, with side-wall `Mesh` for all
/// renderable glyphs in the `text` string.
///
/// Meant to be the backing Mesh for a `StandardMaterial`. Glyph cap faces are
/// triangulated earcut using contour winding to classify outer polygons vs.
/// holes.
pub fn build_text_3d_mesh(
    atlas: &SlugAtlas,
    font_id: FontId,
    text: &str,
    half_adv: f32,
    depth: f32,
    tolerance: f32,
    xy_outset: f32,
) -> Mesh {
    let half_depth = depth * 0.5;

    let Some((ascender, descender)) = atlas.font_metrics(font_id) else {
        return empty_mesh();
    };
    let Some(upm) = atlas.units_per_em(font_id) else {
        return empty_mesh();
    };
    let upm = upm as f32;
    let inv_range = 1.0 / (ascender - descender);
    let outline_type = atlas
        .font_outline_type(font_id)
        .unwrap_or(OutlineType::TrueType);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let mut cursor_x_em: f32 = 0.0;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if let Some(gid) = atlas.glyph_index_for_char(font_id, ch) {
                cursor_x_em += atlas.glyph_advance(font_id, gid).unwrap_or(upm * 0.5);
            } else {
                cursor_x_em += upm * 0.5;
            }
            continue;
        }

        let Some(gid) = atlas.glyph_index_for_char(font_id, ch) else {
            cursor_x_em += upm * 0.5;
            continue;
        };
        let Some(cached) = atlas.cached_glyph(font_id, gid) else {
            cursor_x_em += atlas.glyph_advance(font_id, gid).unwrap_or(upm * 0.5);
            continue;
        };

        // Inset Z so that side-wall faces sit inside the cap planes.
        // TODO: broken like a Vegas gambler.
        let sw_half_depth = half_depth * (1.0 - 0.005);
        {
            let sw_start = positions.len();
            build_glyph_side_walls(
                cached,
                cursor_x_em,
                ascender,
                descender,
                half_adv,
                sw_half_depth,
                tolerance,
                xy_outset,
                &mut positions,
                &mut normals,
                &mut indices,
            );
            // Pad UVs for side-wall vertices we just added. UV = (x, y) of the
            // vertex position on the cap plane handy for future side-wall
            // texturing if I ever go crazy.
            for pos in &positions[sw_start..] {
                uvs.push([pos[0], pos[1]]);
            }
        }

        // Linearize every font curve contour for the glyph into world-space
        // polygons, computing signed area to classify outer vs. hole.
        struct ContourPoly {
            pts: Vec<[f32; 2]>,
            is_hole: bool,
        }

        let mut contour_polys: Vec<ContourPoly> = Vec::new();
        for contour in cached.contours() {
            let mut pts: Vec<[f32; 2]> = Vec::new();
            for curve in contour {
                let sub = adaptive_subdivisions(curve.p0, curve.p1, curve.p2, tolerance);
                for [ex, ey] in linearize_quad(curve.p0, curve.p1, curve.p2, sub) {
                    let wx = (ex + cursor_x_em) * inv_range - half_adv;
                    let wy = (ey - ascender) * inv_range + 0.5;
                    pts.push([wx, wy]);
                }
            }
            if pts.len() < 3 {
                continue;
            }
            let area = signed_area(&pts);
            // Positive signed area = CCW in font y up cords = outer contour.
            // Negative = CW = hole.
            // Winding convention differs by outline format:
            //   TrueType:   outer = CW = negative shoelace area
            //   CFF / CFF2: outer = CCW = positive shoelace area
            let is_hole = match outline_type {
                OutlineType::TrueType => area > 0.0,
                OutlineType::Cff => area < 0.0,
            };
            contour_polys.push(ContourPoly { pts, is_hole });
        }

        // Assign each hole contour to the outer contour that geometrically
        // contains it, using a point-in-polygon test on the hole's first
        // vertex. This is critical for glyphs like `%` which have multiple
        // outer contours where feeding every hole to every outer causes earcut
        // to bridge across the gap between unrelated contours which is busted
        // but looks funny ngl. TODO: I should have added an integration test
        // for this.
        let outers: Vec<&ContourPoly> = contour_polys.iter().filter(|c| !c.is_hole).collect();
        let holes: Vec<&ContourPoly> = contour_polys.iter().filter(|c| c.is_hole).collect();

        // A hole is assigned to the smallest by area outer that contains its
        // probe point. Using the first match incorrectly assigns deeply-nested
        // holes aka for the 'R' inside ® - to be assigned the outermost circle
        // rather than to the R's own outer contour which leads to the R being
        // filled in as ® for some fonts.
        let outer_areas: Vec<f32> = outers.iter().map(|o| signed_area(&o.pts).abs()).collect();
        let mut outer_holes: Vec<Vec<usize>> = vec![Vec::new(); outers.len()];
        for (hi, hole) in holes.iter().enumerate() {
            let probe = hole.pts[0];
            let mut best: Option<(usize, f32)> = None; // (outer_index, area)
            for (oi, outer) in outers.iter().enumerate() {
                if point_in_polygon(probe, &outer.pts) {
                    let area = outer_areas[oi];
                    if best.is_none_or(|(_, best_area)| area < best_area) {
                        best = Some((oi, area));
                    }
                }
            }
            if let Some((oi, _)) = best {
                outer_holes[oi].push(hi);
            }
            // Holes with no containing outer (malformed glyph data) are
            // silently dropped - they can't be triangulated sensibly.
            // TODO: I can't find a font that fits this but I'm sure there is
            // some degenerate font doing it, add an info message here?
        }

        for (oi, outer) in outers.iter().enumerate() {
            let my_holes: Vec<&ContourPoly> = outer_holes[oi].iter().map(|&hi| holes[hi]).collect();

            let mut flat: Vec<f64> = Vec::with_capacity(
                (outer.pts.len() + my_holes.iter().map(|h| h.pts.len()).sum::<usize>()) * 2,
            );
            let mut hole_indices: Vec<usize> = Vec::new();

            for [x, y] in &outer.pts {
                flat.push(*x as f64);
                flat.push(*y as f64);
            }
            for hole in &my_holes {
                hole_indices.push(flat.len() / 2);
                for [x, y] in &hole.pts {
                    flat.push(*x as f64);
                    flat.push(*y as f64);
                }
            }

            let tri_indices = earcutr::earcut(&flat, &hole_indices, 2).unwrap_or_default();
            if tri_indices.is_empty() {
                continue;
            }

            let all_pts: Vec<[f32; 2]> = flat
                .chunks_exact(2)
                .map(|c| [c[0] as f32, c[1] as f32])
                .collect();

            // TrueType outer contours are CW in Y up, so earcut produces CW
            // triangles. Bevy's CCW front-face convention means CW triangles
            // face away from the viewer so we have to reverse the winding or it
            // looks like ass.
            let front_base = positions.len() as u32;
            for [x, y] in &all_pts {
                positions.push([*x, *y, half_depth]);
                normals.push([0.0, 0.0, 1.0]);
                uvs.push([*x, *y]);
            }
            for tri in tri_indices.chunks_exact(3) {
                indices.push(front_base + tri[2] as u32);
                indices.push(front_base + tri[1] as u32);
                indices.push(front_base + tri[0] as u32);
            }

            let back_base = positions.len() as u32;
            for [x, y] in &all_pts {
                positions.push([*x, *y, -half_depth]);
                normals.push([0.0, 0.0, -1.0]);
                uvs.push([*x, *y]);
            }
            for &ti in &tri_indices {
                indices.push(back_base + ti as u32);
            }
        }

        cursor_x_em += atlas.glyph_advance(font_id, gid).unwrap_or(upm * 0.5);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
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
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    m.insert_indices(Indices::U32(Vec::new()));
    m
}
