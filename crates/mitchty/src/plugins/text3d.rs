//! 3-D text plugin.
//!
//! All entity-lifecycle logic now lives in [`flan::SlugText3dPlugin`] / [`flan::text3d`].
//! This file retains only application-specific concerns:
//!
//! - Font loading via the Bevy asset server
//! - Initial visibility (`setup_3d_text` sets `state.visible = true`)
//! - Reverie / world-clock text sync (`sync_text3d_to_active_reverie`)
//! - Re-exports consumed by egui and other mitchty modules

use bevy::prelude::*;

use crate::plugins::fonts::{RegisteredFontEntry, RegisteredFonts};
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName};

pub use flan::{
    SlugText3dApply, SlugText3dFontValidation, SlugText3dState, Text3d, Text3dRenderer,
};

/// Backwards-compat alias.
pub use flan::Text3dFontId as FlanFontId;
pub use flan::Text3dFontId;

/// Handle to the NotoSansJP font, held until the asset is loaded.
#[derive(Resource)]
pub struct Text3dFontHandle(pub Handle<Font>);

/// Startup: kick off async loading of NotoSansJP.
pub fn load_text3d_font(mut commands: Commands, asset_server: Res<AssetServer>) {
    use crate::assets::asset_path;
    let handle = asset_server.load(asset_path("fonts/NotoSansJP-Regular.ttf"));
    commands.insert_resource(Text3dFontHandle(handle));
}

/// Update system once font bytes are loaded, register with `SlugAtlas`, populate
/// `RegisteredFonts` so the egui picker shows it, store `Text3dFontId`, update
/// `state.font_id`, and immediately send `SlugText3dApply` so the entity
/// spawns without waiting for the debounce timer.
pub fn setup_flan_font(
    mut commands: Commands,
    font_handle: Option<Res<Text3dFontHandle>>,
    font_assets: Res<Assets<Font>>,
    mut atlas: ResMut<flan::slug::SlugAtlas>,
    mut state: ResMut<SlugText3dState>,
    mut registered_fonts: ResMut<RegisteredFonts>,
    mut apply_msg: bevy::ecs::message::MessageWriter<SlugText3dApply>,
) {
    let Some(handle) = font_handle else { return };
    let Some(font) = font_assets.get(&handle.0) else {
        return;
    };

    commands.remove_resource::<Text3dFontHandle>();

    match atlas.register_font(font.data.data().to_vec()) {
        Ok(id) => {
            // Resource for the font picker active-font display.
            commands.insert_resource(Text3dFontId(id));
            // Add to RegisteredFonts so the combo-box can show the name.
            // Avoid duplicates in case of hot-reload.
            if !registered_fonts.0.iter().any(|e| e.font_id == id) {
                registered_fonts.0.push(RegisteredFontEntry {
                    name: "NotoSansJP-Regular.ttf".to_string(),
                    font_id: id,
                });
            }
            // Set state - bypasses the debounce so the entity spawns immediately
            // without two separate 150 ms waits.
            state.font_id = Some(id);
            // Fire apply right now so the entity spawns in this frame rather
            // than after another full debounce cycle.
            apply_msg.write(SlugText3dApply);
        }
        Err(e) => {
            bevy::log::error!("SlugText3d: failed to register NotoSansJP font: {e}");
        }
    }
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

    let new_text = if let Some(cd) = countdown_str {
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

    if state.text != new_text {
        state.text = new_text;
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

        app.add_systems(Startup, (setup_3d_text, load_text3d_font))
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
