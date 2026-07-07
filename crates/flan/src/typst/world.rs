use typst::Library;
use typst::LibraryExt as _;
/// A minimal in-memory `typst::World` implementation.
///
/// `SimpleWorld` satisfies the [`typst::World`] trait using only:
/// - a single in-memory source file aka the typst markup string passed by the caller
/// - one or more font byte slices fed directly from [`crate::slug::SlugAtlas`]
///   so that both the typst shaper and the slug renderer operate on the same
///   underlying glyph ID namespace.
///
/// File I/O via `#include`, `#image`, external packages is intentionally not
/// supported for my use cases for now. I can add it in in post.
use typst::foundations::{Bytes, Datetime, Duration, Styles};
use typst::syntax::{FileId, Source};
use typst::text::{Font, FontBook, FontFamily, FontFeatures, FontList, Tag, TextElem};
use typst::utils::LazyHash;
use typst_kit::fonts::FontStore;

/// Build the document-wide default [`Styles`] used by every [`SimpleWorld`].
///
/// 1. Pins `text.font` to `primary_family` the caller-supplied font's own
///    family name, read from its `FontInfo`.
/// 2. Disables OpenType ligature substitution e.g. `liga`/`clig` via
///    [`TextElem::ligatures`]) and contextual alternates `calt`, via the raw
///    [`TextElem::features`] override `ligatures` alone does not work.
///
/// See `crate::typst::bridge::tests` for the regression tests (`officia_...`,
/// `lorem_paragraph_...`) that reproduce and guard both font failure modes here
/// for now. They were more to figure out the issues and figure out what I need
/// to do later on for this.
fn default_styles(primary_family: Option<&str>) -> Styles {
    let mut styles = Styles::new();
    if let Some(family) = primary_family {
        styles.push(TextElem::font.set(FontList(vec![FontFamily::new(family)])));
    }
    styles.push(TextElem::ligatures.set(false));
    styles.push(TextElem::features.set(FontFeatures(vec![(Tag::from_bytes(b"calt"), 0)].into())));
    styles
}

/// Minimal typst world for single-source in-memory compilation.
///
/// Construct with [`SimpleWorld::new`], then pass to [`typst::compile`].
pub struct SimpleWorld {
    /// The typst standard library (routing table for all built-in functions).
    library: LazyHash<Library>,
    /// Font metadata index consumed by the typst shaper.
    fonts: FontStore,
    /// The one source file typst will compile.
    source: Source,
    /// The caller-supplied primary font, kept for comparison against
    /// [`typst::layout::FrameItem::Text::font`] so callers can tell which glyphs
    /// were actually shaped with the primary font vs. one of the embedded
    /// fallback fonts.
    ///
    /// `None` only if `font_bytes` failed to parse.
    primary_font: Option<Font>,
}

impl SimpleWorld {
    /// Create a world that will compile `source_text` using the given font.
    ///
    /// The embedded typst default fonts (Libertinus Serif, New Computer Modern,
    /// DejaVu Sans Mono) are always added as fallbacks so that typst's built-in
    /// elements (rule lines, math, etc.) can render without errors even when
    /// the primary font does not cover every glyph.
    ///
    /// This MVP approach is broken though cause Slug doesn't know about these
    /// fonts yet.
    pub fn new(source_text: String, font_bytes: &[u8], face_index: u32) -> Self {
        let mut fonts = FontStore::new();

        // Register the primary font from the caller-supplied bytes.
        let bytes = Bytes::new(font_bytes.to_vec());
        let primary_font = Font::new(bytes, face_index);
        if let Some(font) = primary_font.clone() {
            let info = font.info().clone();
            fonts.push((font, info));
        }

        // Add embedded typst default fonts as fallbacks.
        // TODO: Need to integrate this stuff with Bevy's Asset system
        fonts.extend(typst_kit::fonts::embedded());

        let primary_family = primary_font.as_ref().map(|f| f.info().family.as_str());
        let mut library = Library::default();
        library.styles = default_styles(primary_family);

        Self {
            library: LazyHash::new(library),
            fonts,
            source: Source::detached(source_text),
            primary_font,
        }
    }

    /// The caller-supplied primary font, for identity comparison against
    /// shaped [`typst::layout::FrameItem::Text`] runs. `None` if `font_bytes`
    /// passed to [`SimpleWorld::new`] failed to parse.
    pub fn primary_font(&self) -> Option<&Font> {
        self.primary_font.as_ref()
    }
}

impl typst::World for SimpleWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> typst::diag::FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(typst::diag::FileError::NotFound(std::path::PathBuf::from(
                "<SimpleWorld: external files not supported>",
            )))
        }
    }

    fn file(&self, _id: FileId) -> typst::diag::FileResult<Bytes> {
        Err(typst::diag::FileError::NotFound(std::path::PathBuf::from(
            "<SimpleWorld: external files not supported>",
        )))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        // Return a fixed date so compilation is deterministic.
        Datetime::from_ymd(2024, 1, 1)
    }
}
