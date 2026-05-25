// modular_bitfield yeets out too many parens so ignore it for this file
// TODO: future me figure out a better approach to layout, maybe reuse the taffy
// layout thing bevy uses?
#![allow(unused_parens)]

// Note: This entire thing is likely a refactor for the future. I need to read
// more UV space layout papers or something to brain up a better api for laying
// out uv space blocks and what that would look like from the rust side.
//
// I'm not a web guy but the css approach is insanely complex and I don't like it.
//
// This approach is "just enough to make me think this isn't implented too dum".
//
// I will need to tackle the hard part of interleaving text with gooey elements
// but thats future mitch problem.
//
// Also for now this is a hack af shader more to experiment with using slugtext
// in a shader itself as well as if I'm honest learn how to mix text with
// graphical fragments in a shader. I put it all off cause texture atlases made
// total ass looking text but slugtext made this all worth starting to learn.
//
// Multi-line string/input contract, aka there is none if you pass in newlines.
//
// This layout system is single-line only. If you pass a string containing
// '\n', everything from the first newline onwards is dropped like a rock as
// far as the shader is concerned. Callers wanting two lines should split
// their string at '\n', compute two separate UV regions manually and make two
// separate slugtext() calls. That split is the caller's responsibility not
// the shader the shader gives zero shits about multi line flow or reflow. I'm
// not going to implement the CSS layout algorithm or whatever TEX does until
// maybe much later.
//
// For now all I need is simple strings/word drawing so skipping the "hard"
// parts seems sensible. Basically if I start dealing with reflow I now need to
// deal with centered text reflow, lhs/rhs etc... and its summer I don't got the
// time for that kinda nitpicky bs.

// Slug text layout descriptor, controls how the text block is positioned and
// scaled within the UV region provided to `slugtext()`. The descriptor is
// packed into 4 bits for alignment on HxW and stored in the
// `SlugTextUniform.layout` u32 field so the shader can decode it cheaply and I
// leave future shenanigans available to the bitfield for Vertical. Horizontal
// uses all 4 bits with Fill. That is explained later.
//
// Note its simply a glorified bitfield generator.
//
// Build a layout with:
//   let l = Layout::new()
//       .with_vertical(Vertical::Top)        = first 2 bits
//       .with_horizontal(Horizontal::Left);  = second 2 bits
//   let as_u32 = l.to_u32();

use modular_bitfield::prelude::*;

/// Vertical alignment of the text block within the UV region.
///
/// Bit encoding layout 2 bits, occupies `layout[1:0]`, default is 0.
///   0 = Center
///   1 = Top
///   2 = Bottom
///   3 = Unused (falls through to Center in the shader)
///
/// With `Center` the text is centered on the vertical uv axis.
/// With `Top` the ascender font line sits at `region.top`.
/// With `Bottom` the descender font line sits at `region.bottom`.
///
/// There is intentionally no `Vertical::Fill` variant. Vertical scale is
/// always derived from the horizontal scale decision in the shader - the H
/// enum owns the scale mode, V only picks the offset anchor. A standalone
/// "fill height" vertical mode has no independent meaning.
///
/// If `Horizontal::Fill` is set, the vertical offset is irrelevant anyway
/// because `glyph_height_uv == region_size.y` by construction.
// TODO: I have zero idea if this makes sense but for now it works good enough
// for governmnent work so I'll ship it even if I think its kinda dum. "I'll fix
// it in post" (I probably won't until I *really* have to)
#[derive(BitfieldSpecifier, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[bits = 2]
pub enum Vertical {
    /// Center the text vertically within the region default.
    #[default]
    Center = 0,
    /// Align the top of the text block with the top of the region.
    Top = 1,
    /// Align the bottom of the text block with the bottom of the region.
    Bottom = 2,
    /// Reserved. Falls through to Center behavior in the shader.
    Unused = 3,
}

// TODO: For now I am assuming/treating all glyph sdf data as if it were fixed
// width basically. Font rendering is way more complect than I expected but I
// don't need to eat that whole elephant at once.
/// Horizontal alignment and scaling of the text block within the UV region.
///
/// Bit encoding is same as Vertical 2 bits, occupies `layout[3:2]`:
///   0 = Center - Default letterbox scale min of (scale_w / scale_h), text
///                block centered horizontally and text never overflows the
///                region just is scaled within it. Never partial fill a region
///                with text, I can't think why you'd want text to only be
///                partially visible in a plot or graph anyway. For my use
///                cases it makes zero sense for now.
///   1 = Left   - fill-height scale em_scale == scale_h, text anchored to
///                the left edge right overflow is clipped by the container.
///   2 = Right  - fill-height scale em_scale == scale_h, text anchored to
///                the right edge; left overflow is clipped by the container.
///   3 = Fill   - each axis scaled independently to fill the region:
///                 em_scale_x = scale_w  (x fills full region width)
///                 em_scale_y = scale_h  (y fills full region height)
///                Aspect ratio is abandoned; glyphs stretch to fill the box.
///                h_offset is always 0.0 (text starts at left edge) and
///                vert_bits are ignored since the text fills the height.
///                Use when the container is explicitly sized to hold the text
///                at a known aspect ratio and distortion is acceptable, e.g.
///                a fixed-size indicator widget or a label whose node was
///                measured to exactly fit the string.
///
/// `Center` keeps glyph SDFs square and ensures the full string is visible.
/// `Left` / `Right` are the natural choice for text labels where the node is
/// sized to hold the text the shader fills the full height and the caller
/// controls width.
/// `Fill` abandons aspect preservation entirely: both axes scale to fit the region.
///
/// Note: `Vertical` does not have a matching Fill variant because the vertical
/// scale is always derived from the horizontal scale. A standalone "fill
/// height" on the vertical axis has no good meaning independent of what height
/// selects. `Vertical::Unused` (3) is reserved and falls through to `Center`
/// behavior in the shader.
#[derive(BitfieldSpecifier, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[bits = 2]
pub enum Horizontal {
    /// Letterbox scale, centered horizontally.
    #[default]
    Center = 0,
    /// Fill-height scale, left-aligned overflowing glyphs clip on the right.
    Left = 1,
    /// Fill-height scale, right-aligned overflowing glyphs clip on the left.
    Right = 2,
    /// Fill both axes independently each axis scales to fill the region. Aspect
    /// ratio is abandoned here and glyphs stretch to fill the containing box.
    Fill = 3,
}

/// 4-bit packed layout descriptor for the slug text shader.
///
/// Current bit layout:
///   bits 1:0 = `Vertical`   0=Center, 1=Top,    2=Bottom, 3=Unused->Center
///   bits 3:2 = `Horizontal` 0=Center, 1=Left,   2=Right,  3=Fill
///
/// The default `Layout::new()` is `Vertical::Center + Horizontal::Center`.
///
/// # Example
/// ```rust
/// use flan::layout::{Layout, Vertical, Horizontal};
///
/// // Left-aligned, vertically centered.
/// let l = Layout::new()
///     .with_vertical(Vertical::Center)
///     .with_horizontal(Horizontal::Left);
/// assert_eq!(l.to_u32(), 0b0100); // horizontal=Left(1) << 2 | vertical=Center(0)
/// ```
#[bitfield(bits = 4)]
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    // Vertical alignment (2 bits, [1:0]).
    #[bits = 2]
    pub vertical: Vertical,
    // Horizontal alignment and scale mode (2 bits, [3:2]).
    #[bits = 2]
    pub horizontal: Horizontal,
}

// Default for a Layout struct is Center vertical and horizontal. Callers need
// to think about what thigns look like.
impl Default for Layout {
    fn default() -> Self {
        Layout::new()
            .with_vertical(Vertical::Center)
            .with_horizontal(Horizontal::Center)
    }
}

impl Layout {
    /// Encode the layout as a `u32` bitfield for uploading as a WGSL uniform.
    pub fn to_u32(self) -> u32 {
        self.into_bytes()[0] as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_centered() {
        let l = Layout::default();
        assert_eq!(l.vertical(), Vertical::Center);
        assert_eq!(l.horizontal(), Horizontal::Center);
        assert_eq!(l.to_u32(), 0);
    }

    // This is where I think its probably better to find a layout engine I can
    // reuse for now. I don't know what a good api would be anyway this is all
    // new to me.
    #[test]
    fn left_top_encodes_correctly() {
        let l = Layout::new()
            .with_vertical(Vertical::Top)
            .with_horizontal(Horizontal::Left);
        assert_eq!(l.to_u32(), 0b0101);
    }

    #[test]
    fn right_bottom_encodes_correctly() {
        let l = Layout::new()
            .with_vertical(Vertical::Bottom)
            .with_horizontal(Horizontal::Right);
        assert_eq!(l.to_u32(), 0b1010);
    }

    #[test]
    fn center_horizontal_left_vertical() {
        let l = Layout::new()
            .with_vertical(Vertical::Center)
            .with_horizontal(Horizontal::Left);
        assert_eq!(l.to_u32(), 0b0100);
    }

    #[test]
    fn fill_encodes_correctly() {
        let l = Layout::new()
            .with_vertical(Vertical::Center)
            .with_horizontal(Horizontal::Fill);
        assert_eq!(l.to_u32(), 0b1100);
    }

    #[test]
    fn vertical_unused_encodes_to_3() {
        let l = Layout::new()
            .with_vertical(Vertical::Unused)
            .with_horizontal(Horizontal::Center);
        assert_eq!(l.to_u32(), 0b0011);
    }

    #[test]
    fn roundtrip_all_valid_combinations() {
        // Vertical::Unused is intentionally excluded from the roundtrip since
        // it has no defined shader behavior. The wgsl shader just falls back to
        // Center in that case but thats not something layout needs to care
        // about.
        for v in [Vertical::Center, Vertical::Top, Vertical::Bottom] {
            for h in [
                Horizontal::Center,
                Horizontal::Left,
                Horizontal::Right,
                Horizontal::Fill,
            ] {
                let l = Layout::new().with_vertical(v).with_horizontal(h);
                let packed = l.to_u32();
                assert_eq!(
                    packed & 0x3,
                    v as u32,
                    "vertical bits wrong for {v:?}+{h:?}"
                );
                assert_eq!(
                    (packed >> 2) & 0x3,
                    h as u32,
                    "horizontal bits wrong for {v:?}+{h:?}"
                );
            }
        }
    }
}
