// TODO: This is a bit of a human tail leftover from work I did that didn't
// last.

//! Font-ID resource shared between flan's 3d text systems and the application.
//!
//! [`Text3dFontId`] holds the [`FontId`] returned by [`crate::slug::SlugAtlas::register_font`].
//! It is inserted by the application's font-loading startup system (e.g.
//! `setup_flan_font` in mitchty) after the font bytes are ready.
//! Flan's [`crate::text3d::SlugText3dPlugin`] reads it from `Option<Res<Text3dFontId>>` so
//! spawning is gracefully deferred when the font is not yet loaded.

use crate::slug::FontId;

/// Holds the [`FontId`] for the active 3d text font.
///
/// Inserted by the application once the font bytes have been loaded and
/// registered with [`crate::slug::SlugAtlas`]. Flan's `manage_text3d`
/// and [`crate::text3d::spawn_text3d`] read it as `Option<Res<Text3dFontId>>`
/// and skip spawning until it is present.
#[derive(bevy::prelude::Resource)]
pub struct Text3dFontId(pub FontId);
