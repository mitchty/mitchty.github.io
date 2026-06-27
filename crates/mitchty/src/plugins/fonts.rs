//! Font management plugin. TODO: probably yeet this into flan.
//!
//! Owns the async font-registration pipeline that is shared across all
//! subsystems 3-D text, FPS overlay, UI, etc. Any code that wants to make
//! a `FontBytes` asset available to the `SlugAtlas` simply spawns a
//! [`PendingFontRegistration`] entity; this plugin handles the rest.
//!
//! ## Registration flow
//!
//! 1. Caller spawns an entity with [`PendingFontRegistration`] attached.
//! 2. [`register_pending_fonts`] runs whenever such entities exist
//!    (`any_with_component` gate). For each entity it polls the asset server;
//!    if the bytes are loaded it calls `SlugAtlas::register_font`, sends a
//!    [`FontRegistered`] message, and despawns the entity. Not-yet-loaded
//!    entities are left for the next frame.
//! 3. [`apply_font_registered`] reads [`FontRegistered`] messages and appends
//!    de-duplicated entries to [`RegisteredFonts`].
//! 4. Any UI or system that needs the font list reads [`RegisteredFonts`].

use bevy::prelude::*;

/// A named font that has been successfully registered with the `SlugAtlas`.
#[derive(Clone, Debug)]
pub struct RegisteredFontEntry {
    /// Human-readable label shown in picker UI (e.g. `"NotoSansJP-Regular.ttf"`).
    pub name: String,
    /// Opaque atlas identifier returned by `SlugAtlas::register_font`.
    pub font_id: flan::slug::FontId,
}

/// All fonts successfully registered with the `SlugAtlas`, in insertion order.
///
/// Read by UI code to populate font-picker combo-boxes. Never written
/// directly by UI - use [`PendingFontRegistration`] to add new entries.
#[derive(Resource, Default)]
pub struct RegisteredFonts(pub Vec<RegisteredFontEntry>);

/// Work-item component.
///
/// Spawn an entity carrying this component to request that the named font be
/// registered with the `SlugAtlas` once its bytes have finished loading.
/// The entity is automatically despawned by [`register_pending_fonts`] once
/// the registration attempt completes (success *or* failure).
#[derive(Component)]
pub struct PendingFontRegistration {
    /// Display name used as the combo-box label and de-duplication key.
    pub name: String,
    /// Strong handle that keeps the asset alive until registration completes.
    pub handle: Handle<Font>,
}

/// Sent by [`register_pending_fonts`] after a font is successfully registered.
///
/// [`apply_font_registered`] consumes this to append an entry to
/// [`RegisteredFonts`].
#[derive(Clone)]
pub struct FontRegistered {
    pub name: String,
    pub font_id: flan::slug::FontId,
}
impl bevy::ecs::message::Message for FontRegistered {}

/// Poll pending font-registration entities and register any whose bytes have
/// loaded.
///
/// Gated by `any_with_component::<PendingFontRegistration>` so it is
/// completely idle on frames where the queue is empty.
///
/// For each entity:
/// - Asset not yet loaded -> skip; entity remains for the next frame.
/// - Asset loaded -> call `atlas.register_font`; on success send
///   [`FontRegistered`]; log error on failure; despawn the entity either way.
pub fn register_pending_fonts(
    pending_q: Query<(Entity, &PendingFontRegistration)>,
    font_assets: Res<Assets<Font>>,
    mut atlas: ResMut<flan::slug::SlugAtlas>,
    mut font_registered: bevy::ecs::message::MessageWriter<FontRegistered>,
    mut commands: Commands,
) {
    for (entity, pending) in pending_q.iter() {
        let Some(font) = font_assets.get(&pending.handle) else {
            continue;
        };
        match atlas.register_font(font.data.data().to_vec()) {
            Ok(font_id) => {
                bevy::log::info!("fonts: registered '{}' as {:?}", pending.name, font_id);
                font_registered.write(FontRegistered {
                    name: pending.name.clone(),
                    font_id,
                });
            }
            Err(e) => {
                bevy::log::error!("fonts: failed to register '{}': {e}", pending.name);
            }
        }
        // Despawn whether or not registration succeeded - a broken font should
        // not be retried on every subsequent frame.
        commands.entity(entity).despawn();
    }
}

/// Consume [`FontRegistered`] messages and append de-duplicated entries to
/// [`RegisteredFonts`].
pub fn apply_font_registered(
    mut events: bevy::ecs::message::MessageReader<FontRegistered>,
    mut registered: ResMut<RegisteredFonts>,
) {
    for ev in events.read() {
        if registered.0.iter().any(|e| e.name == ev.name) {
            continue;
        }
        registered.0.push(RegisteredFontEntry {
            name: ev.name.clone(),
            font_id: ev.font_id,
        });
    }
}

/// Plugin that owns the font-registration pipeline.
///
/// Must be added before any plugin that spawns [`PendingFontRegistration`]
/// entities or reads [`RegisteredFonts`].
pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegisteredFonts>()
            .add_message::<FontRegistered>()
            .add_systems(
                Update,
                register_pending_fonts.run_if(any_with_component::<PendingFontRegistration>),
            )
            .add_systems(Update, apply_font_registered.after(register_pending_fonts));
    }
}
