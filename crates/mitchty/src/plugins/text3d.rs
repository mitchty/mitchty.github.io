//! 3-D text plugin.
//!
//! All entity-lifecycle logic now lives in [`flan::SlugText3dPlugin`] / [`flan::text3d`].
//! This file retains only application-specific concerns:
//!
//! - Font loading via the Bevy asset server
//! - Initial visibility (`setup_3d_text` sets `state.visible = true`)
//! - Reverie / world-clock text sync (`sync_text3d_to_active_reverie`)
//! - Re-exports consumed by egui and other mitchty modules
//! - Typst demo entity spawn (feature = "typst" on flan, this is a complete work in progress)

use bevy::prelude::*;

use crate::plugins::fonts::RegisteredFonts;
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName};

pub use flan::{
    SlugText3dApply, SlugText3dFontValidation, SlugText3dState, Text3d, Text3dRenderer,
};

/// Backwards-compat alias.
pub use flan::Text3dFontId as FlanFontId;
pub use flan::Text3dFontId;

/// Sentinel resource: present while `setup_flan_font` still needs to run.
#[derive(Resource, Default)]
pub struct Text3dFontHandle;

/// The font name this plugin manages for 3-D slug text.
///
/// Kept as NotoSansJP-Regular.ttf because [`flan::text3d::SlugText3dState`]'s
/// default text is `"mitchty 美智君"` and it needs kanji glyph coverage that
/// FiraMono doesn't have. I did try NotoSansJP-Regular.ttf but it appears to
/// have a broken `liga` GSUB substitution for Latin "ffi" but the reverie
/// *content* rendering path for latin text has been switched to FiraMono from
/// this.
const SLUG_3D_FONT_NAME: &str = "NotoSansJP-Regular.ttf";

/// Update system to wait until NotoSansJP has been registered by the
/// [`FontsPlugin`] pipeline, then:
/// * Reuses the already-registered [`FontId`] from [`RegisteredFonts`] does
///   not call `atlas.register_font()` again.
/// * Stores [`Text3dFontId`] and updates [`SlugText3dState`].
/// * Fires [`SlugText3dApply`] so the 3-D slug entity spawns immediately.
/// * Marks [`ActiveReverie`] as changed so `sync_typst_reverie_view` re-runs
///   if a reverie was already selected before the font was ready.
/// * Spawns the typst demo entity if needed/called for by the cli
///
/// The system removes itself from the schedule via `commands.remove_resource`
/// once the font is found.
pub fn setup_flan_font(
    mut commands: Commands,
    registered_fonts: Res<RegisteredFonts>,
    mut state: ResMut<SlugText3dState>,
    mut apply_msg: bevy::ecs::message::MessageWriter<SlugText3dApply>,
    mut active_reverie: ResMut<crate::plugins::reveries::ActiveReverie>,
) {
    // Wait until FontsPlugin has finished registering NotoSansJP.
    let Some(entry) = registered_fonts
        .0
        .iter()
        .find(|e| e.name == SLUG_3D_FONT_NAME)
    else {
        // Font bytes not yet registered so try again next frame.
        return;
    };

    let id = entry.font_id;

    // Drop the load token so this system stops running again.
    commands.remove_resource::<Text3dFontHandle>();

    // Publish the authoritative FontId resource.
    commands.insert_resource(Text3dFontId(id));

    // Update slug-text state and fire apply immediately (bypasses 150 ms
    // debounce so the 3-D entity spawns in this frame).
    state.font_id = Some(id);
    apply_msg.write(SlugText3dApply);

    // Touch ActiveReverie so sync_typst_reverie_view re-fires if a reverie was
    // already selected before this font was ready. Really only relevant at
    // startup.
    active_reverie.set_changed();

    bevy::log::info!(
        "text3d: using {:?} ({SLUG_3D_FONT_NAME}) for 3-D slug text",
        id
    );
}

/// Startup: make the 3-D text visible by default.
pub fn setup_3d_text(mut state: ResMut<SlugText3dState>) {
    state.visible = true;
}

/// Once per second: update `state.text` from the active world-clock countdown
/// or reverie name, falling back to `state.default_text`.
pub fn sync_text3d_to_active_reverie(
    active_post: Res<ActiveReverie>,
    display_q: Query<&ReverieDisplayName>,
    world_clock: Option<Res<crate::ui::WorldClockState>>,
    mut state: ResMut<SlugText3dState>,
    // Track the last text we set so we can compare without touching state
    // (ResMut marks the resource changed on access via DerefMut, but a plain
    // field read through Deref doesn't). We do the compare via this Local and
    // only call state.text = ... when there's an actual change, which is the
    // only operation that goes through DerefMut and triggers is_changed().
    mut last_text: Local<Option<String>>,
) {
    use jiff::Timestamp;
    let now = Timestamp::now();

    let countdown_str: Option<String> = world_clock.and_then(|wc| {
        wc.alarms
            .iter()
            .filter(|a| a.target_ts > now)
            .min_by_key(|a| a.target_ts.as_second())
            .map(|a| {
                let secs = (a.target_ts.as_second() - now.as_second()).max(0) as u64;
                let cd =
                    humantime::format_duration(std::time::Duration::from_secs(secs)).to_string();
                match &a.label {
                    Some(lbl) => format!("{} in {}", lbl, cd),
                    None => cd,
                }
            })
    });

    let new_text: String = if let Some(cd) = countdown_str {
        cd
    } else {
        match active_post.0 {
            Some(entity) => display_q
                .get(entity)
                .map(|d| d.0.to_string())
                .unwrap_or_else(|_| state.default_text.clone()),
            None => state.default_text.clone(),
        }
    };

    // Compare against our Local cache firstand only write when the text
    // genuinely changed.
    let changed = last_text.as_deref() != Some(new_text.as_str());
    if changed {
        state.text = new_text.clone();
        *last_text = Some(new_text);
    }
}

/// Debug "feature" specific log asset/entity counts once per second.
#[cfg(feature = "debug")]
fn debug_asset_counts(
    meshes: Res<Assets<Mesh>>,
    std_mats: Res<Assets<bevy::pbr::StandardMaterial>>,
    slug_mats: Res<Assets<flan::SlugText3dTextureMaterial>>,
    images: Res<Assets<Image>>,
    buffers: Res<Assets<bevy::render::storage::ShaderBuffer>>,
    entities: Query<Entity>,
) {
    bevy::log::info!(
        "[debug] assets mesh:{} std_mat:{} slug_mat:{} image:{} ssbo:{} | entities:{}",
        meshes.len(),
        std_mats.len(),
        slug_mats.len(),
        images.len(),
        buffers.len(),
        entities.iter().count(),
    );
}

pub struct Text3dPlugin;

impl Plugin for Text3dPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<flan::SlugPlugin>() {
            app.add_plugins(flan::SlugPlugin);
        }
        app.add_plugins(flan::SlugText3dPlugin);

        // Typst demo simply registers the TypstTextPlugin so TypstTextNode entities
        // are compiled and meshed. The actual demo entity is spawned in
        // setup_flan_font once the font is ready.
        app.add_plugins(flan::typst_text::TypstTextPlugin);

        app.init_resource::<Text3dFontHandle>();
        app.add_systems(Startup, setup_3d_text)
            .add_systems(
                Update,
                setup_flan_font.run_if(resource_exists::<Text3dFontHandle>),
            )
            .add_systems(
                Update,
                sync_text3d_to_active_reverie.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
            );

        #[cfg(feature = "debug")]
        app.add_systems(
            Update,
            debug_asset_counts.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(1),
            )),
        );
    }
}
