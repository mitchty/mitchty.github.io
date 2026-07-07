/// Convert a typst source document into per-glyph layout spans.
///
/// This is the bridge between typst's layout engine and the slug shader
/// pipeline basically. It is woefully incomplete and barely works for simple
/// typst docs. I will only expand it as far as I need it.
use typst::layout::{Frame, FrameItem, Point};
use typst::text::Font;
use typst_layout::PagedDocument;

use super::world::SimpleWorld;

/// A single shaped glyph as positioned by typst's layout engine.
///
/// Coordinates are in typst's page space: origin at top-left, x increases
/// right, y increases downward. The unit is in typst points which are 1 pt =
/// 1/72 inch.
///
/// `x_pt` is the origin of the glyph left edge of the glyph advance, already
/// accounting for kerning and x_offset from rustybuzz shaping. `y_pt` is the
/// baseline y glyph position.
#[derive(Debug, Clone)]
pub struct TypstGlyphSpan {
    /// OpenType glyph ID produced by rustybuzz shaping. This ID is in the
    /// same namespace as ttf-parser's `GlyphId` so it can be looked up
    /// directly via [`crate::slug::SlugAtlas`].
    pub glyph_id: u16,
    /// Horizontal glyph origin in typst points.
    pub x_pt: f64,
    /// Baseline y position in typst points.
    pub y_pt: f64,
    /// Font size for this glyph's text run, in typst points.
    pub font_size_pt: f64,
    /// Horizontal advance in typst points.
    pub advance_pt: f64,
}

/// Compile `source_text` with typst using `font_bytes` as the primary font,
/// then walk the resulting frame tree to collect per-glyph layout spans.
///
/// Returns an empty `Vec` on compilation errors diagnostics are logged via
/// `bevy::log::warn` for now, refactor should be a tree with Result's most
/// likely for better error handling.
pub fn layout_typst(source_text: &str, font_bytes: &[u8]) -> Vec<TypstGlyphSpan> {
    let world = SimpleWorld::new(source_text.to_owned(), font_bytes, 0);
    let result = typst::compile::<PagedDocument>(&world);

    if !result.warnings.is_empty() {
        for w in &result.warnings {
            bevy::log::warn!("typst warning: {}", w.message);
        }
    }

    let doc = match result.output {
        Ok(doc) => doc,
        Err(errors) => {
            for e in &errors {
                bevy::log::error!("typst error: {}", e.message);
            }
            return Vec::new();
        }
    };

    let Some(primary_font) = world.primary_font() else {
        bevy::log::error!(
            "layout_typst: primary font failed to parse from font_bytes, \
             cannot identify which shaped glyphs belong to this layout so producing \
             no spans rather than risking garbled output"
        );
        return Vec::new();
    };

    let mut spans = Vec::new();
    for page in doc.pages() {
        walk_frame(&page.frame, Point::zero(), primary_font, &mut spans);
    }
    spans
}

/// Recursively walk a [`Frame`], collecting [`TypstGlyphSpan`]s.
///
/// `origin` is the accumulated offset of the current frame relative to the
/// page origin, in typst's absolute coordinate system.
///
/// `primary_font` is the [`Font`] data the slug atlas has glyph
/// outlines cached for. Any [`FrameItem::Text`] whose
/// `text_item.font.font()` does not compare equal to it is silently skipped:
/// those glyphs were shaped with a typst fallback font (e.g. Libertinus
/// Serif, used when the primary font lacks coverage for some character) and
/// their glyph IDs belong to a *different* face's namespace, so including
/// them would corrupt the atlas lookup and produce garbled glyphs.
///
/// TODO: This is the biggest next refactor step for this MVP. I need to
/// refactor flan entirely anyway so future me task.
fn walk_frame(frame: &Frame, origin: Point, primary_font: &Font, out: &mut Vec<TypstGlyphSpan>) {
    for (pos, item) in frame.items() {
        let abs = Point {
            x: origin.x + pos.x,
            y: origin.y + pos.y,
        };

        match item {
            FrameItem::Text(text_item) => {
                // Skip text runs shaped with a fallback font. Their glyph IDs
                // come from a different face's glyph table and would produce
                // wrong glyphs if looked up in the slug atlas for the primary
                // font. This all exposed Flan needs its own shared Font
                // subsystem for Slug Text and this or other things. I don't
                // know what that looks like yet though.
                if text_item.font.font() != primary_font {
                    bevy::log::debug!(
                        "walk_frame: skipping {} glyph(s) from fallback font {:?} \
                         (primary is {:?})",
                        text_item.glyphs.len(),
                        text_item.font.font(),
                        primary_font,
                    );
                    continue;
                }

                let font_size = text_item.size;
                let font_size_pt = font_size.to_pt();

                // Walk glyphs left to right, accumulating x from advances.
                let mut cursor_x = abs.x;
                let baseline_y = abs.y;

                for glyph in &text_item.glyphs {
                    // x_offset is the shaper's per-glyph kerning nudge.
                    let x_offset = glyph.x_offset.at(font_size);
                    let x_pt = cursor_x.to_pt() + x_offset.to_pt();
                    let y_pt = baseline_y.to_pt();

                    let advance = glyph.x_advance.at(font_size);
                    let advance_pt = advance.to_pt();

                    out.push(TypstGlyphSpan {
                        glyph_id: glyph.id,
                        x_pt,
                        y_pt,
                        font_size_pt,
                        advance_pt,
                    });

                    cursor_x += advance;
                }
            }

            FrameItem::Group(group) => {
                // Groups may carry a transform. For the MVP we only handle pure
                // translation for now. Full affine transforms would require
                // propagating a matrix through the walk so skip that complexity
                // for future sucker winter mitch.
                let group_origin = Point {
                    x: abs.x + group.transform.tx,
                    y: abs.y + group.transform.ty,
                };
                walk_frame(&group.frame, group_origin, primary_font, out);
            }

            // Shapes, images, links, tags - not text, nothing to collect at this point.
            // TODO: Add support for images and links at least. Links'll be
            // "fun" not sure how to let people click on a link in 3d...
            // bevy_picking and raycasting?
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slug::SlugAtlas;
    use ttf_parser::Face;

    const FIRA_TTF: &[u8] = include_bytes!("../../tests/fixtures/FiraMono-Medium.ttf");
    const NOTO_TTF: &[u8] = include_bytes!("../../tests/fixtures/NotoSansJP-Regular.ttf");

    fn layout(text: &str) -> Vec<TypstGlyphSpan> {
        layout_typst(text, FIRA_TTF)
    }

    fn assert_one_glyph_per_char(font_bytes: &[u8], text: &str, font_label: &str) {
        let spans = layout_typst(text, font_bytes);
        let expected_count = text.chars().count();

        assert_eq!(
            spans.len(),
            expected_count,
            "{font_label}: shaping {text:?} produced {} span(s), expected {} \
             an OpenType feature in typst appears to be \
             merging chars into a ligature/substitution glyph",
            spans.len(),
            expected_count,
        );
    }

    fn assert_glyph_ids_match_direct_char_mapping(font_bytes: &[u8], text: &str, font_label: &str) {
        let face = Face::parse(font_bytes, 0).expect("font parse");
        let spans = layout_typst(text, font_bytes);
        let expected: Vec<char> = text.chars().collect();

        assert_eq!(
            spans.len(),
            expected.len(),
            "{font_label}: span/char count mismatch for {text:?}"
        );

        for (span, &ch) in spans.iter().zip(expected.iter()) {
            let direct_gid = face
                .glyph_index(ch)
                .unwrap_or_else(|| panic!("{font_label}: {ch:?} has no direct glyph mapping"));
            assert_eq!(
                span.glyph_id, direct_gid.0,
                "{font_label}: shaped glyph_id {} for char {:?} does not match \
                 direct char->glyph mapping {} likely a broken GSUB \
                 substitution ligature/contextual-alternate producing the \
                 wrong glyph for another font",
                span.glyph_id, ch, direct_gid.0,
            );
        }
    }

    /// This is kinda temporary for now until I do a refactor to integrate
    /// typsts fonts more broadly.
    fn assert_all_runs_use_primary_font(font_bytes: &[u8], text: &str, font_label: &str) {
        let world = super::super::world::SimpleWorld::new(text.to_owned(), font_bytes, 0);
        let result = typst::compile::<PagedDocument>(&world);
        let doc = result
            .output
            .unwrap_or_else(|e| panic!("{font_label}: compile failed: {e:?}"));
        let primary = world
            .primary_font()
            .unwrap_or_else(|| panic!("{font_label}: primary font failed to parse"))
            .clone();

        fn walk(frame: &Frame, primary: &Font, font_label: &str, reconstructed: &mut String) {
            for (_, item) in frame.items() {
                match item {
                    FrameItem::Text(ti) => {
                        assert!(
                            ti.font.font() == primary,
                            "{font_label}: text run {:?} was shaped with fallback \
                             font {:?} instead of the primary font {:?} typst's \
                             default text.font selection is leaking through again",
                            ti.text,
                            ti.font.font(),
                            primary,
                        );
                        reconstructed.push_str(&ti.text);
                    }
                    FrameItem::Group(g) => walk(&g.frame, primary, font_label, reconstructed),
                    _ => {}
                }
            }
        }

        let mut reconstructed = String::new();
        for page in doc.pages() {
            walk(&page.frame, &primary, font_label, &mut reconstructed);
        }

        assert_eq!(
            reconstructed, text,
            "{font_label}: reconstructed text from shaped runs does not match input"
        );
    }

    #[test]
    fn layout_returns_nonempty_spans_for_ascii() {
        let spans = layout("hello");
        assert!(
            !spans.is_empty(),
            "ascii text must produce at least one span"
        );
    }

    #[test]
    fn layout_returns_empty_for_empty_source() {
        let spans = layout("");
        assert!(spans.is_empty());
    }

    #[test]
    fn spans_have_positive_font_size() {
        let spans = layout("abc");
        for s in &spans {
            assert!(
                s.font_size_pt > 0.0,
                "every span must carry a positive font size"
            );
        }
    }

    #[test]
    fn spans_have_positive_advance() {
        let spans = layout("abc");
        for s in &spans {
            assert!(
                s.advance_pt > 0.0,
                "every span must carry a positive advance"
            );
        }
    }

    /// "officia" is chosen to test here because typst historically triggers an
    /// `ff` ligature to be shaped with a fallback font when the primary font
    /// lacks it. This is really a gap in this renderer but I only realized that
    /// after debugging this too much. Most of this test suites really part of
    /// debugging render issues with this mvp.
    #[test]
    fn all_glyph_ids_exist_in_primary_font() {
        let face = Face::parse(FIRA_TTF, 0).expect("FiraMono parse");
        let spans = layout("officia");

        assert!(
            !spans.is_empty(),
            "layout_typst must produce spans for 'officia'"
        );

        for span in &spans {
            // ttf-parser stores glyph outlines by GlyphId; if the ID came from
            // a fallback font it will either be absent or point to a completely
            // different glyph in another fonts data.
            let gid = ttf_parser::GlyphId(span.glyph_id);
            // The glyph must at least *exist* in FiraMono. Abuse
            // glyph_hor_advance as the cheapest presence check where every
            // glyph a font knows about has one.
            let advance = face.glyph_hor_advance(gid);
            assert!(
                advance.is_some(),
                "glyph_id {} from layout_typst is not in FiraMono's glyph table \
                 likely a fallback-font ID that leaked through somehow",
                span.glyph_id,
            );
        }
    }

    #[test]
    fn all_glyph_ids_exist_in_primary_font_lorem() {
        let face = Face::parse(FIRA_TTF, 0).expect("FiraMono parse");
        let spans =
            layout("Lorem ipsum dolor sit amet, officia deserunt mollit anim id est laborum.");
        assert!(!spans.is_empty());
        for span in &spans {
            let gid = ttf_parser::GlyphId(span.glyph_id);
            assert!(
                face.glyph_hor_advance(gid).is_some(),
                "glyph_id {} is not in FiraMono fallback-font ID leak prolly",
                span.glyph_id,
            );
        }
    }

    /// This is here to have a quicker way to detect that `build_run_from_spans`
    /// doesn't end up in a retry loop with nothing rendered in the world.
    #[test]
    fn shaped_glyph_ids_round_trip_through_atlas() {
        let spans = layout("officia");
        assert!(!spans.is_empty());

        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font");

        let glyph_ids: Vec<u16> = spans.iter().map(|s| s.glyph_id).collect();
        atlas.validate_glyph_ids(fid, &glyph_ids);
        atlas.build_frame_atlas(&[(fid, glyph_ids.clone())]);

        for &gid in &glyph_ids {
            // Whitespace / no-outline glyphs are not added to the frame atlas
            // intentionally that is expected and correct; only check glyphs
            // that actually have outlines.
            if atlas.cached_glyph(fid, gid).is_some() {
                assert!(
                    atlas.frame.glyph_index(fid, gid).is_some(),
                    "glyph_id {} has an outline in the cache but is absent from \
                     the frame atlas atlas build bug prolly",
                    gid,
                );
            }
        }
    }

    /// Same round-trip for the full Lorem ipsum sentence that triggered a atlas bug
    #[test]
    fn lorem_glyph_ids_round_trip_through_atlas() {
        let text = "Lorem ipsum dolor sit amet, officia deserunt mollit anim id est laborum.";
        let spans = layout(text);
        assert!(!spans.is_empty());

        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(FIRA_TTF.to_vec())
            .expect("register_font");

        let mut glyph_ids: Vec<u16> = spans.iter().map(|s| s.glyph_id).collect();
        glyph_ids.sort_unstable();
        glyph_ids.dedup();

        atlas.validate_glyph_ids(fid, &glyph_ids);
        atlas.build_frame_atlas(&[(fid, glyph_ids.clone())]);

        for &gid in &glyph_ids {
            if atlas.cached_glyph(fid, gid).is_some() {
                assert!(
                    atlas.frame.glyph_index(fid, gid).is_some(),
                    "glyph_id {} is cached but missing from frame atlas",
                    gid,
                );
            }
        }
    }

    #[test]
    fn typst_glyph_ids_exist_in_primary_font_namespace() {
        let face = Face::parse(FIRA_TTF, 0).expect("FiraMono parses");
        let spans = layout("Hello");

        assert!(
            !spans.is_empty(),
            "must produce at least one span for 'Hello'"
        );

        for span in &spans {
            let gid = ttf_parser::GlyphId(span.glyph_id);
            assert!(
                face.glyph_hor_advance(gid).is_some(),
                "glyph_id {} from layout_typst has no advance in FiraMono \
                 wrong font namespace or fallback-font leak? prolly bug again",
                span.glyph_id,
            );
        }
    }

    /// For a single line of left-to-right text, x coordinates must be
    /// non-decreasing such that each glyph starts at or after the previous glyph.
    #[test]
    fn span_x_coordinates_are_non_decreasing_for_ltr() {
        let spans = layout("abcdef");
        assert!(spans.len() >= 2, "need at least 2 spans to check ordering");
        for window in spans.windows(2) {
            assert!(
                window[1].x_pt >= window[0].x_pt,
                "x_pt must be non-decreasing for LTR text: {} < {}",
                window[1].x_pt,
                window[0].x_pt,
            );
        }
    }

    #[test]
    fn officia_does_not_ligature_collapse_in_noto_sans_jp() {
        assert_one_glyph_per_char(NOTO_TTF, "officia", "NotoSansJP");
        assert_glyph_ids_match_direct_char_mapping(NOTO_TTF, "officia", "NotoSansJP");
    }

    #[test]
    fn officia_does_not_ligature_collapse_in_fira_mono() {
        assert_one_glyph_per_char(FIRA_TTF, "officia", "FiraMono");
    }

    /// Classic Latin ligature-prone letter clusters "fi", "fl", "ffl", "tt".
    /// Broader net than just "officia" in case a future font or a future typst
    /// upgrade has a different broken GSUB lookup that only triggers on one of
    /// these but not "ffi". I need a better option here. Not sure what though
    /// so keeping the tests for future me to build a better future.
    #[test]
    fn common_ligature_clusters_do_not_collapse_in_noto_sans_jp() {
        for word in ["fi", "fine", "fly", "waffle", "attic"] {
            assert_one_glyph_per_char(NOTO_TTF, word, "NotoSansJP");
            assert_glyph_ids_match_direct_char_mapping(NOTO_TTF, word, "NotoSansJP");
        }
    }

    const LOREM_PARAGRAPH_1: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.";

    #[test]
    fn all_runs_use_primary_font_for_lorem_paragraph_fira_mono() {
        assert_all_runs_use_primary_font(FIRA_TTF, LOREM_PARAGRAPH_1, "FiraMono");
    }

    #[test]
    fn all_runs_use_primary_font_for_lorem_paragraph_noto_sans_jp() {
        assert_all_runs_use_primary_font(NOTO_TTF, LOREM_PARAGRAPH_1, "NotoSansJP");
    }

    #[test]
    fn all_runs_use_primary_font_for_short_ascii_text() {
        for text in ["hello", "officia", "abcdef"] {
            assert_all_runs_use_primary_font(FIRA_TTF, text, "FiraMono");
            assert_all_runs_use_primary_font(NOTO_TTF, text, "NotoSansJP");
        }
    }

    #[test]
    fn lorem_paragraph_one_glyph_per_char_fira_mono() {
        assert_one_glyph_per_char(FIRA_TTF, LOREM_PARAGRAPH_1, "FiraMono");
    }

    #[test]
    fn lorem_paragraph_one_glyph_per_char_noto_sans_jp() {
        assert_one_glyph_per_char(NOTO_TTF, LOREM_PARAGRAPH_1, "NotoSansJP");
    }

    #[test]
    fn officia_sentence_round_trips_correctly_through_noto_sans_jp_atlas() {
        let text = "similique sunt in culpa qui officia deserunt mollitia animi, \
                     id est laborum et dolorum fuga.";

        assert_one_glyph_per_char(NOTO_TTF, "officia", "NotoSansJP");
        assert_glyph_ids_match_direct_char_mapping(NOTO_TTF, "officia", "NotoSansJP");

        let spans = layout_typst(text, NOTO_TTF);
        assert!(!spans.is_empty());

        let mut atlas = SlugAtlas::default();
        let fid = atlas
            .register_font(NOTO_TTF.to_vec())
            .expect("register_font");

        let mut glyph_ids: Vec<u16> = spans.iter().map(|s| s.glyph_id).collect();
        glyph_ids.sort_unstable();
        glyph_ids.dedup();

        atlas.validate_glyph_ids(fid, &glyph_ids);
        atlas.build_frame_atlas(&[(fid, glyph_ids.clone())]);

        for &gid in &glyph_ids {
            if atlas.cached_glyph(fid, gid).is_some() {
                assert!(
                    atlas.frame.glyph_index(fid, gid).is_some(),
                    "glyph_id {} is cached but missing from frame atlas",
                    gid,
                );
            }
        }
    }
}
