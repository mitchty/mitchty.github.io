mod ai;
mod assets;
mod fullscreen_effect;
mod mesh_effect;
mod post_process;
mod ui;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
#[cfg(feature = "feathers")]
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;

use bevy_pretty_text::prelude::*;

use crate::post_process::{EffectsEnabled, PostProcessSettings};
use mitchty::RenderLayers;

// TODO: maybe a shared gooey lib?
pub use mitchty::CameraMode;

/// Resource to hold the current background color state.
/// `None` is "whatever the dark/light theme uses"
/// `Some(color)` means user color
#[derive(Resource)]
pub struct ColorState {
    pub color: Option<Srgba>,
}

/// Marker component for the current GLTF/GLB scene entity so it can be
/// despawned and respawned when a different file is picked.
#[derive(Component)]
pub struct LoadedScene;

/// Tracks the loaded GLB/GLTF scene.
///
/// `custom_scene` holds whatever string the user supplied uri or what probably
///  should be a Path. Either passed directly to `AssetServer::load`, which
///  routes them to the correct `AssetSource` for me so I don't need to do
///  anything special. Uses `FileAssetReader` for paths, `WebAssetReader` for
///  URLs.
///
/// `None` here just means use the embedded at compile time GLB model.
#[derive(Resource, Default)]
pub struct SceneConfig {
    pub custom_scene: Option<String>,
}

/// State for the Load Scene URL popup window.
#[derive(Resource, Default)]
pub struct SceneUrlState {
    /// If the popup is currently open, might want to make this no blocking
    pub open: bool,
    /// Text buffer bound to the URL `TextEdit`.
    pub buf: String,
}

use polars::prelude::*;
use rand::RngExt;

use assets::{AssetConfigPlugin, asset_path};
use bevy_fontmesh::prelude::*;
use flan::shaders::ShadersPlugin;
use fullscreen_effect::{
    CameraConfig, CameraOrbit, manage_effect_settings, next_effect, previous_effect,
    toggle_fullscreen_effect, update_effect_time,
};
use mesh_effect::MeshEffectPlugin;
use post_process::PostProcessPlugin;
use ui::{ScrollViewPlugin, SettingsUiPlugin, ToggleCameraProjection, send_scroll_events};

/// Absolute rotation speed
const SPEED: f32 = 2.25;
/// Minimum rotation speed in radians per second
const MIN_SPEED: f32 = -SPEED;
/// Maximum rotation speed in radians per second
const MAX_SPEED: f32 = SPEED;
// Retaining for future weekend abuse
// Golden angle for rotation calculations
//const GOLDEN_ANGLE: f32 = 137.507_77;

/// Marker component for entities that should rotate
#[derive(Component)]
struct Rotator {
    /// Base rotation speed in radians per second for each axis (x, y, z)
    base_speed: Vec3,
}

/// Marker component to indicate cube rotation is enabled (on cube entities)
#[derive(Component)]
pub struct CubeRotationEnabled;

/// Marker component separate from cube entities to control rotation
#[derive(Component, Default)]
pub struct CubeRotation;

/// Marker component to indicate hue animation is enabled (on cube entities)
#[derive(Component)]
pub struct HueAnimationEnabled;

/// Marker component separate from cube entities to control hue animation
#[derive(Component, Default)]
pub struct HueAnimation;

/// Marker component for the FPS text entity
#[derive(Component)]
struct FpsText;

/// Marker component to indicate FPS should be displayed and updated
#[derive(Component, Default)]
pub struct FpsDisplay;

/// Marker component to indicate camera should be rotating
#[derive(Component)]
pub struct CameraRotationEnabled;

/// Marker component for the main camera to enable TV effect toggling
#[derive(Component)]
pub struct MainCamera;

/// Resource to track mouse and touch drag state for free-look camera
#[derive(Resource, Default)]
pub struct DragState {
    /// Are we actively dragging or not?
    pub is_dragging: bool,
    /// Starting position when mouse button was pressed or touch started
    pub drag_start: Option<Vec2>,
    /// Current mouse/touch position
    pub current_pos: Vec2,
    /// Total distance dragged from start
    pub drag_distance: f32,
    /// Active touch ID iff dragging
    pub active_touch_id: Option<u64>,
    /// Previous position for calculating deltas between drags
    pub previous_pos: Option<Vec2>,
    /// Second finger touch ID for two-finger zoom
    pub secondary_touch_id: Option<u64>,
    /// Current position of the second finger
    pub secondary_touch_pos: Option<Vec2>,
    /// Distance between the two fingers on the previous frame for pinch delta
    pub previous_pinch_distance: Option<f32>,
}

/// Camera free-look component
#[derive(Component, Clone, Copy)]
pub struct FreeLookCamera {
    /// Yaw in radians
    pub yaw: f32,
    /// Pitch in radians
    pub pitch: f32,
    /// Sensitivity?
    pub sensitivity: f32,
}

/// Marker component indicating free-look is currently active (blocks automatic rotation)
/// Stores the last interaction time so that auto rotate can resume if on
#[derive(Component)]
struct FreeLookActive(f64);

/// Marker component for the initial touch help overlay mostly for web stuff
#[derive(Component)]
pub struct DisplayInitialHelp;

/// cli arguments, note for wasm I need to find a way to get clap to map to
/// /uri/paths and params I can yeet into the binary as an equivalent.
#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser, Debug)]
#[command(about = "mitchty - just me playing around for funsies", long_version = lib::build_info::VERSTR)]
struct Cli {
    /// Enable gamepad support
    #[arg(long, overrides_with = "no_gamepad")]
    with_gamepad: bool,

    // Note: I added this vs default features false and whatnot to not have to
    // deal with running this in wine on linux and having an xbox controller
    // cause a panic in a dependency of a dependency.
    /// Disable gamepad support, default
    #[arg(long = "no-gamepad", overrides_with = "with_gamepad")]
    no_gamepad: bool,

    /// Open one or more app windows at startup. Comma separated or repeated
    /// args allowed. Known values: world-clock, recognizer, data-viewer.
    #[arg(long, value_delimiter = ',', value_name = "APP", action = clap::ArgAction::Append)]
    app: Vec<String>,

    /// Open a specific post by name at startup. Note the post text is matched
    /// case insensitively to make the wasm uri case symmetric with the command
    /// line ars.
    #[arg(long, value_name = "POST")]
    post: Option<String>,

    /// Override the world clock timezone list. Comma-separated or repeated inputs allowed.
    /// Each value must be a valid IANA timezone name e.g. America/New_York.
    #[arg(long = "tz", value_delimiter = ',', value_name = "TZ", action = clap::ArgAction::Append)]
    tz: Vec<String>,

    /// Set up the initial world clock alarms.
    /// Format: `IANA_TZ:UTC_SECONDS` or `LABEL:IANA_TZ:UTC_SECONDS`, label is optional.
    /// UTC_SECONDS is the Unix epoch timestamp in seconds when the alarm fires.
    /// Example: --alarm America/New_York:1893456000
    /// Example: --alarm "Birthday:America/New_York:1893456000"
    #[arg(long = "alarm", value_name = "[LABEL:]TZ:EPOCH", action = clap::ArgAction::Append)]
    alarm: Vec<String>,

    /// Set the initial sort column for the World Clock table.
    /// Values: timezone, time, date, offset, delta-local
    #[arg(long = "sort", value_name = "COLUMN")]
    sort: Option<String>,

    /// Set the initial sort direction for the World Clock table.
    /// Values: asc (default), desc
    #[arg(long = "sort-dir", value_name = "DIR")]
    sort_dir: Option<String>,

    /// Start the World Clock frozen at this UTC moment instead of live time.
    /// Value is a Unix timestamp in seconds since epoch.
    #[arg(long = "pinned", value_name = "EPOCH")]
    pinned: Option<i64>,
}

/// Minimal percent-decoder for URL query string values I need. If I do more
/// this should be dropped like a rock and a crate found.
///
/// Only handles `%XX` sequences and `+` space. Good enough for IANA tz names
/// which can contain `/` encoded as `%2F` and epoch seconds which is all I need
/// for now.
#[cfg(target_arch = "wasm32")]
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((hi * 16 + lo) as u8 as char);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(lpModuleName: *const u8) -> *const c_void;
    fn GetProcAddress(hModule: *const c_void, lpProcName: *const u8) -> *const c_void;
}

#[cfg(target_os = "windows")]
fn is_wine() -> bool {
    unsafe {
        let kernel32 = GetModuleHandleA("kernel32.dll\0".as_ptr());
        if kernel32.is_null() {
            return false;
        }
        let wine_func = GetProcAddress(kernel32, "wine_get_unix_file_name\0".as_ptr());
        !wine_func.is_null()
    }
}

/// TODO: This files getting obscenely too long time to start splitting stuff up
/// into a dedicated lib crate. I've put this off for too long.
// ALso need to start moving arg parsing into dedicated functions and add unit tests.
fn main() {
    // Set up better panic messages for WASM for when this stuff seems to not
    // work or I manage to use a library that won't run on it without paying
    // attention... again.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    // This is actually just for running under wine so that jiff isn't confused
    // and looks for a zoneinfo file that may not be there and the timezones
    // don't work the way you'd want. On actual windows this is basically a nop.
    #[cfg(target_os = "windows")]
    bevy::log::info!("is_wine {}", is_wine());

    #[cfg(target_os = "windows")]
    if is_wine() {
        unsafe {
            std::env::remove_var("TZDIR");
        }
        bevy::log::info!("wine workaround, removed TZDIR for time parsing");
    }

    // Native: parse CLI args and build a UiConfig from them.
    #[cfg(not(target_arch = "wasm32"))]
    let (enable_gamepad, ui_config) = {
        use clap::Parser;
        use jiff::Timestamp;
        let cli = Cli::parse();

        let mut cfg = ui::UiConfig::default();
        for slug in &cli.app {
            match ui::UiWindow::from_slug(slug) {
                Some(w) => cfg.enable_window(w),
                None => bevy::log::warn!("--app: unknown app {:?} ignoring it", slug),
            }
        }

        if let Some(name) = &cli.post {
            match ui::post_index_for_name(name) {
                Some(idx) => cfg.initial_post = Some(idx),
                None => bevy::log::warn!("--post: unknown post {:?} ignoring it", name),
            }
        }

        // Collect --tz values. Validate each name against the bundled tz
        // database and skip unknown inputs assuming they're people being
        // cheeky. Stuff not in the tzdb is dropped like a rock.
        for tz in &cli.tz {
            let tz = tz.trim();
            if tz.is_empty() {
                continue;
            }
            if jiff_tzdb::available().any(|n| n == tz) {
                cfg.initial_timezones.push(tz.to_string());
            } else {
                bevy::log::warn!("--tz: unknown timezone {:?} ignoring it", tz);
            }
        }

        // Parse --alarm [LABEL:]TZ:EPOCH_SECS entries.
        for entry in &cli.alarm {
            match ui::world_clock::parse_alarm_entry(entry) {
                Some((secs, tz, label)) => match Timestamp::from_second(secs) {
                    Ok(ts) => cfg.initial_alarms.push((ts, tz, label)),
                    Err(_) => {
                        bevy::log::warn!("--alarm: epoch out of range in {:?} ignoring it", entry)
                    }
                },
                None => bevy::log::warn!(
                    "--alarm: expected [LABEL:]TZ:EPOCH format, got {:?} ignoring it",
                    entry
                ),
            }
        }

        // Parse --sort column.
        if let Some(col_slug) = &cli.sort {
            use ui::world_clock::SortColumn;
            match SortColumn::from_slug(col_slug.trim()) {
                Some(col) => cfg.initial_sort_col = col,
                None => bevy::log::warn!("--sort: unknown column {:?} ignoring it", col_slug),
            }
        }

        // Parse --sort-dir direction.
        if let Some(dir_slug) = &cli.sort_dir {
            use ui::world_clock::SortDir;
            match SortDir::from_slug(dir_slug.trim()) {
                Some(dir) => cfg.initial_sort_dir = dir,
                None => bevy::log::warn!(
                    "--sort-dir: expected asc or desc, got {:?} ignoring it",
                    dir_slug
                ),
            }
        }

        // Parse --pinned epoch seconds.
        if let Some(secs) = cli.pinned {
            match Timestamp::from_second(secs) {
                Ok(ts) => cfg.initial_pinned = Some(ts),
                Err(_) => bevy::log::warn!("--pinned: epoch out of range {} ignoring it", secs),
            }
        }

        (cli.with_gamepad, cfg)
    };

    // WASM: no CLI, use the URL query string as a pseudo cli command arg.
    #[cfg(target_arch = "wasm32")]
    let enable_gamepad = false;

    // WASM: parse ?app=recognizer or ?app=world-clock,recognizer from the
    // browser URL. web_sys gives us location.search() which returns the raw
    // query string including the leading '?'.
    #[cfg(target_arch = "wasm32")]
    let ui_config = {
        use jiff::Timestamp;
        let mut cfg = ui::UiConfig::default();

        let query = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default();

        for pair in query.trim_start_matches('?').split('&') {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim();
            let raw_value = parts.next().unwrap_or("").trim();
            // Percent-decode the value so spaces/special chars survive URL encoding.
            let value = percent_decode(raw_value);
            let value = value.as_str();

            if key.eq_ignore_ascii_case("app") {
                // Value may itself be comma-separated just like in the cli. Symmetry is nice.
                for slug in value.split(',') {
                    let slug = slug.trim();
                    if slug.is_empty() {
                        continue;
                    }
                    match ui::UiWindow::from_slug(slug) {
                        Some(w) => cfg.enable_window(w),
                        None => bevy::log::warn!("?app=: unknown app {:?} ignoring it", slug),
                    }
                }
            } else if key.eq_ignore_ascii_case("post") {
                // Single post name; only one post can be active at a time, last one wins.
                match ui::post_index_for_name(value) {
                    Some(idx) => cfg.initial_post = Some(idx),
                    None => bevy::log::warn!("?post=: unknown post {:?} ignoring it", value),
                }
            } else if key.eq_ignore_ascii_case("tz") {
                // Comma-separated list of IANA timezone names. Validate each
                // against the bundled tz database and skip unknowns.
                // TODO: This all needs a refactor but thats for later me.
                for tz in value.split(',') {
                    let tz = tz.trim();
                    if tz.is_empty() {
                        continue;
                    }
                    if jiff_tzdb::available().any(|n| n == tz) {
                        cfg.initial_timezones.push(tz.to_string());
                    } else {
                        bevy::log::warn!("?tz=: unknown timezone {:?} ignoring it", tz);
                    }
                }
            } else if key.eq_ignore_ascii_case("alarm") {
                // Each value is [LABEL:]TZ:EPOCH_SECS. May be repeated or comma-separated.
                for entry in value.split(',') {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        continue;
                    }
                    match ui::world_clock::parse_alarm_entry(entry) {
                        Some((secs, tz, label)) => match Timestamp::from_second(secs) {
                            Ok(ts) => cfg.initial_alarms.push((ts, tz, label)),
                            Err(_) => bevy::log::warn!(
                                "?alarm=: epoch out of range in {:?} ignoring it",
                                entry
                            ),
                        },
                        None => bevy::log::warn!(
                            "?alarm=: expected [LABEL:]TZ:EPOCH, got {:?} ignoring it",
                            entry
                        ),
                    }
                }
            } else if key.eq_ignore_ascii_case("sort") {
                use ui::world_clock::SortColumn;
                match SortColumn::from_slug(value) {
                    Some(col) => cfg.initial_sort_col = col,
                    None => bevy::log::warn!("?sort=: unknown column {:?} ignoring it", value),
                }
            } else if key.eq_ignore_ascii_case("sort-dir") {
                use ui::world_clock::SortDir;
                match SortDir::from_slug(value) {
                    Some(dir) => cfg.initial_sort_dir = dir,
                    None => bevy::log::warn!(
                        "?sort-dir=: expected asc or desc, got {:?} ignoring it",
                        value
                    ),
                }
            } else if key.eq_ignore_ascii_case("pinned") {
                match value.parse::<i64>() {
                    Ok(secs) => match Timestamp::from_second(secs) {
                        Ok(ts) => cfg.initial_pinned = Some(ts),
                        Err(_) => {
                            bevy::log::warn!("?pinned=: epoch out of range {:?} ignoring it", value)
                        }
                    },
                    Err(_) => bevy::log::warn!(
                        "?pinned=: expected integer epoch, got {:?} ignoring it",
                        value
                    ),
                }
            }
        }

        cfg
    };

    let mut app = App::new();

    app.add_plugins(assets::create_default_plugins(enable_gamepad))
        .add_plugins(AssetConfigPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(ShadersPlugin)
        .add_plugins(FontMeshPlugin::<StandardMaterial>::default())
        .add_plugins(PostProcessPlugin)
        .add_plugins(MeshEffectPlugin)
        .add_plugins(PrettyTextPlugin)
        .add_plugins(flan::PlotPlugin)
        .insert_resource(flan::PlotDataFrame {
            df: initial_plot_df(),
        })
        .insert_resource(flan::SparklineDataFrame {
            df: DataFrame::empty(),
        })
        .init_resource::<FpsHistory>()
        .init_resource::<CameraMode>()
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
        .insert_resource(ColorState { color: None })
        .insert_resource(ui_config)
        .init_resource::<CameraConfig>()
        .init_resource::<Text3dContent>();

    app.init_resource::<SceneConfig>()
        .init_resource::<SceneUrlState>()
        .add_systems(Update, replace_scene);

    // Conditionally add UI-specific plugins
    #[cfg(feature = "egui")]
    {
        // egui renders via render graph nodes, not cameras, so it automatically
        // renders on top of both 3D and 2D cameras
        app.add_plugins(bevy_egui::EguiPlugin::default());
    }

    #[cfg(feature = "feathers")]
    {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()));
    }

    app.add_plugins(SettingsUiPlugin)
        .add_plugins(ScrollViewPlugin)
        .init_resource::<DragState>()
        .add_systems(
            Startup,
            (
                setup,
                setup_fps_ui,
                setup_fps_sparkline_ui,
                setup_3d_text,
                spawn_camera,
            ),
        )
        // Each timed system gets its own add_systems call so their on_timer
        // closures hold completely independent Timer state and can never
        // interfere with each other.
        //
        // Some weird stuff happens when run_if ... on_timer conditions hit the
        // same durations. Mostly it seems like the systems start
        // clobbering/shadowing each other.
        .add_systems(
            Update,
            update_fps_ui.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_millis(500),
            )),
        )
        .add_systems(
            Update,
            tick_plot_data.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_millis(100),
            )),
        )
        .add_systems(
            Update,
            sample_fps_history.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_millis(100),
            )),
        )
        .add_systems(
            Update,
            sync_text3d_to_active_post.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(1),
            )),
        )
        .add_systems(
            Update,
            (
                sync_color_state_to_clear_color,
                track_input_drag,
                free_look_camera,
                cleanup_free_look_after_inactivity,
                zoom_camera,
                animate_materials.run_if(any_with_component::<HueAnimationEnabled>),
                rotate_entities.run_if(any_with_component::<CubeRotationEnabled>),
                toggle_fps_display,
                toggle_cube_rotation,
                toggle_hue_animation,
                toggle_fullscreen_effect,
                next_effect,
                previous_effect,
                apply_cube_rotation,
                apply_hue_animation,
                manage_effect_settings,
                update_effect_time,
                send_scroll_events,
                toggle_camera_projection,
            ),
        );

    // Touch help overlay
    app.add_systems(Startup, setup_help_text)
        .add_systems(Update, dismiss_help_on_input);

    app.run();
}

/// Spawn the single scene camera.
///
/// Of note now, I tried keeping two cameras in sync. Oof was that ass never
/// again. So just swap the Projection mode instead. Its waaaay simpler of a way
/// to approach things. Migrated this from fullscreen_effect. Also probably
/// about time for a refactor of stuff in general in this whole shebang.
///
// Also egui camera usage is... interesting in that it uses the cameras at
// plugin initialization time in startup. Changing that after is... Yeah
// probably not worth worrying about. Will be nice to do all this gooey code in
// the bevy ecs once the bsn is up to snuff so all these weird edge cases are no
// more.
fn spawn_camera(
    mut commands: Commands,
    config: Res<CameraConfig>,
    asset_server: Res<AssetServer>,
    effects_enabled: Res<EffectsEnabled>,
) {
    let diffuse_path = asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    let specular_path = asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");
    let intensity = if effects_enabled.0 { 1.0 } else { 0.0 };

    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            ..default()
        },
        config.transform,
        EnvironmentMapLight {
            diffuse_map: asset_server.load(&diffuse_path),
            specular_map: asset_server.load(&specular_path),
            intensity: 2_000.0,
            ..default()
        },
        config.free_look,
        config.orbit,
        crate::MainCamera,
        PostProcessSettings {
            intensity,
            time: 0.0,
            #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
            _webgl2_padding: Vec2::ZERO,
        },
        RenderLayers::layer(0),
    ));
}

fn setup(asset_server: Res<AssetServer>, mut commands: Commands) {
    // gltf model for testing abuse
    let glb_path = asset_path("mitchty.glb");
    // Use GltfAssetLabel::Scene(0) to load the first scene from the GLB/TF file
    let scene_handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(glb_path));

    // Spawn the GLTF/GLB scene with a large scale to fit the view. Future work
    // will make it possible to set some of this dynamically at load time.
    commands.spawn((
        SceneRoot(scene_handle),
        Transform::from_scale(Vec3::splat(0.3)),
        LoadedScene,
    ));
}

/// Watches `SceneConfig` for changes and swaps out the active scene.
fn replace_scene(
    config: Res<SceneConfig>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
    scene_query: Query<Entity, With<LoadedScene>>,
) {
    // Work around race type issues where GLTF mesh entities loading in the same frame/tick.
    //
    // `is_changed()` is true on the very first frame a resource is spawned
    // otherwise the gltf loader which spawned in initial scene entities and
    // removed them in the same frame can cause panic()s
    if !config.is_changed() || config.is_added() {
        return;
    }

    for entity in scene_query.iter() {
        commands.entity(entity).despawn();
    }

    let path = match &config.custom_scene {
        Some(s) => s.clone(),
        None => asset_path("mitchty.glb"),
    };

    let scene_handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(path));
    commands.spawn((
        SceneRoot(scene_handle),
        Transform::from_scale(Vec3::splat(0.3)),
        LoadedScene,
    ));
}

/// Cube material animation system
fn animate_materials(
    material_handles: Query<&MeshMaterial3d<StandardMaterial>, With<HueAnimationEnabled>>,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for material_handle in material_handles.iter() {
        if let Some(material) = materials.get_mut(material_handle)
            && let Color::Hsla(ref mut hsla) = material.base_color
        {
            *hsla = hsla.rotate_hue(time.delta_secs() * 100.0);
        }
    }
}

// DEAD MAN WALKING! How much do I really want to keep this hack code?
/// Cube random rotation system
fn rotate_entities(
    mut query: Query<(&mut Transform, &mut Rotator), With<CubeRotationEnabled>>,
    time: Res<Time>,
) {
    let mut rng = rand::rng();
    let delta = time.delta_secs();

    for (mut transform, mut rotator) in &mut query {
        let change_x = rng.random_range(-0.1..=0.1);
        let change_y = rng.random_range(-0.1..=0.1);
        let change_z = rng.random_range(-0.1..=0.1);

        rotator.base_speed.x += change_x;
        rotator.base_speed.y += change_y;
        rotator.base_speed.z += change_z;

        if rotator.base_speed.x > MAX_SPEED {
            rotator.base_speed.x = MAX_SPEED - (rotator.base_speed.x - MAX_SPEED);
        }
        if rotator.base_speed.y > MAX_SPEED {
            rotator.base_speed.y = MAX_SPEED - (rotator.base_speed.y - MAX_SPEED);
        }
        if rotator.base_speed.z > MAX_SPEED {
            rotator.base_speed.z = MAX_SPEED - (rotator.base_speed.z - MAX_SPEED);
        }

        if rotator.base_speed.x < MIN_SPEED {
            rotator.base_speed.x = MIN_SPEED + (MIN_SPEED - rotator.base_speed.x);
        }
        if rotator.base_speed.y < MIN_SPEED {
            rotator.base_speed.y = MIN_SPEED + (MIN_SPEED - rotator.base_speed.y);
        }
        if rotator.base_speed.z < MIN_SPEED {
            rotator.base_speed.z = MIN_SPEED + (MIN_SPEED - rotator.base_speed.z);
        }

        transform.rotate_x(rotator.base_speed.x * delta);
        transform.rotate_y(rotator.base_speed.y * delta);
        transform.rotate_z(rotator.base_speed.z * delta);
    }
}

/// System to spawn the fps text entity in the upper right of screen
fn setup_fps_ui(mut commands: Commands) {
    // When the egui menu bar is active squeeze the FPS readout down to avoid
    // overlap.
    #[cfg(feature = "egui")]
    let top = Val::Px(40.0);
    #[cfg(not(feature = "egui"))]
    let top = Val::Px(10.0);

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::srgb(0.0, 1.0, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            top,
            right: Val::Px(10.0),
            ..default()
        },
        FpsText,
    ));
}

/// Spawn the initial touch help overlay.
fn setup_help_text(mut commands: Commands) {
    commands.spawn((
        pretty!("[[Touch and pan to rotate](red, scramble(30, always), shake)]\n[[Touch or click to display a menubar](white, shake(20))]\n[[G/g key also toggles the menubar](yellow, shake(20))]"),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::WHITE),
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(80.0),
            left: Val::Percent(10.0),
            top: Val::Percent(50.0),
            ..default()
        },
        DisplayInitialHelp,
    ));
}

/// Remove the help overlay when some interaction has occurred then nuke the
/// marker component so it never shows again.
fn dismiss_help_on_input(
    mut touch_events: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    help_query: Query<Entity, With<DisplayInitialHelp>>,
    mut commands: Commands,
) {
    let touched = touch_events.read().next().is_some();
    let clicked = mouse.get_just_pressed().next().is_some();

    if touched || clicked {
        for entity in help_query.iter() {
            commands.entity(entity).despawn();
        }
    }
}

// TODO: Here too, I'm thinking its about time to remove some of the keybinding nonsense
/// Send a `ToggleCameraProjection` message to toggle the camera projection
/// m toggles perspective/orthographic
fn toggle_camera_projection(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut events: MessageWriter<ToggleCameraProjection>,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyM) {
        events.write(ToggleCameraProjection);
    }
}

/// Toggle FpsDisplay marker component to control systems that display the fps text
/// f toggles on/off
fn toggle_fps_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyF) {
        if let Ok(entity) = fps_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(FpsDisplay);
        }
    }
}

/// FPS overlay system sparkline and fps text bevy ui node.
///
/// When `FpsDisplay` marker is absent both are hidden, when present the text
/// shows the current FPS and the sparkline strip is visible. Its dataframe is
/// always updated.
fn update_fps_ui(
    diagnostics: Res<DiagnosticsStore>,
    fps_display_query: Query<(), With<FpsDisplay>>,
    mut fps_text_query: Query<&mut Text, With<FpsText>>,
    mut sparkline_query: Query<&mut Visibility, (With<flan::SparklineUiNode>, Without<FpsText>)>,
) {
    let show = !fps_display_query.is_empty();

    let target_vis = if show {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut vis in sparkline_query.iter_mut() {
        if *vis != target_vis {
            *vis = target_vis;
        }
    }

    if !show {
        for mut text in fps_text_query.iter_mut() {
            if !text.0.is_empty() {
                text.0.clear();
            }
        }
        return;
    }

    let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.value())
    else {
        return;
    };

    for mut text in fps_text_query.iter_mut() {
        text.0 = format!("{:.1} fps", fps);
    }
}

/// Number of FPS samples retained 100 samples x 100 ms is about 10 s of history
/// which should be good enough for government work.
const FPS_HISTORY_SAMPLES: usize = 100;

/// Preallocated circular buffer of recent FPS measurements.
///
/// Slots start as `None` and fill in as samples arrive, so the sparkline only
/// draws the portion of history that has actual data. Once all slots are filled
/// the oldest sample is overwritten on each push like a circular buffer.
#[derive(Resource)]
struct FpsHistory {
    data: [Option<f32>; FPS_HISTORY_SAMPLES],
    /// Index of the *next* slot to write to
    index: usize,
}

impl Default for FpsHistory {
    fn default() -> Self {
        Self {
            data: [None; FPS_HISTORY_SAMPLES],
            index: 0,
        }
    }
}

impl FpsHistory {
    /// Write `fps` into the current slot and advance the write cursor, wrapping
    /// around when the end of the buffer is reached so the dataframes like a
    /// cicrular buffer. There is probably a more polars way to do this.
    // TODO: future mitch learn polars better stop being a hack and using an index offset.
    fn push(&mut self, fps: f32) {
        self.data[self.index] = Some(fps);
        self.index = (self.index + 1) % FPS_HISTORY_SAMPLES;
    }

    /// Return the filled samples in chronological order old to new with
    /// normalized data so the maximum observed value maps to `1.0`, the shader
    /// only knows of data from 0 to 1 anyway. This design isn't ideal, its me
    /// being lazy.
    ///
    /// Slots holding `None` are skipped, so the returned slice only covers the
    /// history that has actually been recorded so far. With weird errors being
    /// ignored like an elephant in the room.
    fn to_normalized_values(&self) -> Vec<f32> {
        // Read starting at `index` into the dataframe and wrap around to `index - 1`
        let raw: Vec<f32> = (0..FPS_HISTORY_SAMPLES)
            .map(|offset| (self.index + offset) % FPS_HISTORY_SAMPLES)
            .filter_map(|slot| self.data[slot])
            .collect();

        if raw.is_empty() {
            return Vec::new();
        }

        // Peak fps observed for scaling the entire dataframe for shader display.
        let max_fps = self
            .data
            .iter()
            .filter_map(|v| *v)
            .fold(f32::NEG_INFINITY, f32::max);

        let scale = if max_fps > 0.0 { max_fps } else { 1.0 };

        // This is weird but bevys coord system is annoying, basically invert
        // the y so bevy shows 60fps lower than 120fps.
        raw.iter()
            .map(|&fps| 1.0 - (fps / scale).clamp(0.0, 1.0))
            .collect()
    }
}

/// Spawn the FPS sparkline UI node.
///
/// This is a compact `PlotUiMaterial` as a bevy ui node.
/// `sync_fps_sparkline_visibility` shows/hides it in step with `FpsDisplay`.
fn setup_fps_sparkline_ui(
    mut commands: Commands,
    mut ui_materials: ResMut<Assets<flan::PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<
        Assets<bevy::render::storage::ShaderStorageBuffer>,
    >,
) {
    #[cfg(not(feature = "webgl"))]
    let points_binding = buffers.add(bevy::render::storage::ShaderStorageBuffer::from(
        Vec::<Vec2>::new(),
    ));

    #[cfg(feature = "webgl")]
    let points_binding = flan::PlotPointsUniform {
        data: [Vec4::ZERO; flan::MAX_PLOT_POINTS],
    };

    let material = ui_materials.add(flan::PlotUiMaterial {
        params: flan::PlotUniform {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            zoom: Vec2::ONE,
            offset: Vec2::ZERO,
            count: 0,
            time: 0.0,
            line_width: 0.01,
            #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
            _webgl2_padding: 0.0,
        },
        points: points_binding,
    });

    // Mirror the top offset used by the FPS text label.
    #[cfg(feature = "egui")]
    let top = Val::Px(40.0);
    #[cfg(not(feature = "egui"))]
    let top = Val::Px(10.0);

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            // Sit just to the left of the fps text.
            // fps text: right:10px, ~90px wide → left edge ~100px from screen right.
            // Sparkline: width 100px, with a small gap → right:110px.
            right: Val::Px(140.0),
            top,
            width: Val::Px(100.0),
            height: Val::Px(40.0),
            ..default()
        },
        Visibility::Hidden,
        MaterialNode(material),
        flan::SparklineUiNode,
    ));
}

/// Sample the current FPS into the `FpsHistory` circular buffer every 0.1 s.
///
/// Reads from `DiagnosticsStore` (fed by `FrameTimeDiagnosticsPlugin`) so no
/// `Res<Time>` is needed — the `on_timer` run condition owns the 100 ms cadence.
/// Skips quietly if diagnostics aren't populated yet (first few frames).
fn sample_fps_history(
    diagnostics: Res<DiagnosticsStore>,
    mut fps_history: ResMut<FpsHistory>,
    mut sparkline_df: ResMut<flan::SparklineDataFrame>,
) {
    let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.value())
    else {
        return;
    };

    fps_history.push(fps as f32);

    let values = fps_history.to_normalized_values();
    let n = values.len();
    // Mutating sparkline_df triggers Bevy change detection, which causes
    // flan's sync_sparkline_data to run on the next frame automatically.
    sparkline_df.df = DataFrame::new(n, vec![Column::new("y".into(), values)]).unwrap_or_default();
}

/// Toggle cube rotation marker
/// c toggles on/off
fn toggle_cube_rotation(
    keyboard: Res<ButtonInput<KeyCode>>,
    rotation_query: Query<Entity, With<CubeRotation>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyC) {
        if let Ok(entity) = rotation_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(CubeRotation);
        }
    }
}

fn apply_cube_rotation(
    rotation_marker: Query<(), With<CubeRotation>>,
    cube_query: Query<(Entity, Has<CubeRotationEnabled>), With<Rotator>>,
    mut commands: Commands,
) {
    let should_rotate = !rotation_marker.is_empty();

    for (entity, has_rotation) in cube_query.iter() {
        if should_rotate && !has_rotation {
            commands.entity(entity).insert(CubeRotationEnabled);
        } else if !should_rotate && has_rotation {
            commands.entity(entity).remove::<CubeRotationEnabled>();
        }
    }
}

/// Toggle hue animation
/// h toggles on/off
fn toggle_hue_animation(
    keyboard: Res<ButtonInput<KeyCode>>,
    hue_query: Query<Entity, With<HueAnimation>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_keyboard {
        return;
    }
    if keyboard.just_pressed(KeyCode::KeyH) {
        if let Ok(entity) = hue_query.single() {
            commands.entity(entity).despawn();
        } else {
            commands.spawn(HueAnimation);
        }
    }
}

/// Apply or remove HueAnimationEnabled component toggle
fn apply_hue_animation(
    hue_marker: Query<(), With<HueAnimation>>,
    cube_query: Query<(Entity, Has<HueAnimationEnabled>), With<Rotator>>,
    text3d_query: Query<(Entity, Has<HueAnimationEnabled>), With<Text3d>>,
    mut commands: Commands,
) {
    let should_animate = !hue_marker.is_empty();

    for (entity, has_animation) in cube_query.iter().chain(text3d_query.iter()) {
        if should_animate && !has_animation {
            commands.entity(entity).insert(HueAnimationEnabled);
        } else if !should_animate && has_animation {
            commands.entity(entity).remove::<HueAnimationEnabled>();
        }
    }
}

/// Resource that holds the string displayed as the 3D extruded text above the cubes.
/// Mutate this from any system to change the label at runtime.
#[derive(Resource)]
pub struct Text3dContent(pub String);

impl Default for Text3dContent {
    fn default() -> Self {
        Self(String::from("mitchty.github.io"))
    }
}

/// Sync the 3D text label and update the entity backing it
///
/// Priority order:
/// 1. If the World Clock has at least one active (non-expired) alarm, show the
///    countdown to the soonest one, e.g. `"3h 25m 10s"`.
/// 2. Otherwise, if a blog post is selected, show the post title.
/// 3. Otherwise fall back to `"mitchty.github.io"`.
fn sync_text3d_to_active_post(
    active_post: Res<ui::ActivePost>,
    world_clock: Option<Res<ui::WorldClockState>>,
    mut text_content: ResMut<Text3dContent>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    existing_text: Query<(Entity, &mut Text3d)>,
) {
    use jiff::Timestamp;

    let now = Timestamp::now();

    // Find the soonest active alarm across all world-clock alarms, if any.
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
            Some(idx) => ui::POSTS
                .get(idx)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| String::from("mitchty.github.io")),
            None => String::from("mitchty.github.io"),
        }
    };

    if text_content.0 != new_text {
        let new_text_clone = new_text.clone();

        text_content.0 = new_text;

        // Despawn existing text entities
        let entities_to_despawn: Vec<Entity> =
            existing_text.iter().map(|(entity, _)| entity).collect();

        for entity in entities_to_despawn {
            commands.entity(entity).despawn();
        }

        // Spawn new text immediately after despawning for next tick
        spawn_text3d(
            &mut commands,
            &asset_server,
            &mut materials,
            &new_text_clone,
        );
    }
}

#[derive(Component)]
struct Text3d;

// Spawn the 3d text with whatevers in the Resource string
fn spawn_text3d(
    commands: &mut Commands,
    asset_server: &AssetServer,
    materials: &mut Assets<StandardMaterial>,
    text: &str,
) {
    //    let font_path = asset_path("fonts/FiraMono-Medium.ttf");
    let font_path = asset_path("fonts/NotoSansJP-Regular.ttf");

    let text_material = materials.add(StandardMaterial {
        base_color: Color::from(Hsla::hsl(180.0, 1.0, 0.5)),
        metallic: 0.5,
        perceptual_roughness: 0.3,
        ..default()
    });

    commands.spawn((
        TextMeshBundle {
            text_mesh: TextMesh {
                text: text.to_string(),
                font: asset_server.load(font_path),
                style: TextMeshStyle {
                    depth: 0.1,
                    subdivision: 20,
                    anchor: TextAnchor::Center,
                    justify: JustifyText::Center,
                },
            },
            material: MeshMaterial3d(text_material),
            transform: Transform::from_xyz(0.0, 0.7, 0.0).with_scale(Vec3::splat(0.5)),
            ..default()
        },
        HueAnimationEnabled,
        Text3d,
    ));
}

/// Setup default 3d text for startup
fn setup_3d_text(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    text_content: Res<Text3dContent>,
) {
    spawn_text3d(
        &mut commands,
        &asset_server,
        &mut materials,
        &text_content.0,
    );
}

/// Track mouse and touch drag state for distinguishing clicks from drags
fn track_input_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<CursorMoved>,
    mut touch_events: MessageReader<TouchInput>,
    mut drag_state: ResMut<DragState>,
    interaction_query: Query<&Interaction>,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    let ui_is_interacted = interaction_query
        .iter()
        .any(|interaction| *interaction != Interaction::None);

    // Check if egui is using the input
    #[cfg(feature = "egui")]
    let egui_is_using_input = egui_wants_input.wants_pointer;
    #[cfg(not(feature = "egui"))]
    let egui_is_using_input = false;

    if ui_is_interacted || egui_is_using_input {
        drag_state.is_dragging = false;
        drag_state.drag_start = None;
        drag_state.active_touch_id = None;
        drag_state.previous_pos = None;
        return;
    }

    if mouse.just_pressed(MouseButton::Left) {
        drag_state.is_dragging = true;
        drag_state.drag_start = Some(drag_state.current_pos);
        drag_state.previous_pos = Some(drag_state.current_pos);
        drag_state.drag_distance = 0.0;
        drag_state.active_touch_id = None; // Clear touch if mouse is used
    }

    if mouse.just_released(MouseButton::Left) {
        drag_state.is_dragging = false;
        drag_state.drag_start = None;
        drag_state.previous_pos = None;
    }

    for event in motion.read() {
        drag_state.current_pos = event.position;

        if drag_state.is_dragging
            && let Some(start) = drag_state.drag_start
        {
            drag_state.drag_distance = start.distance(drag_state.current_pos);
        }
    }

    // Track touch input tooooooo TODO: stop copy/pasting lazy past mitch and do it right
    for event in touch_events.read() {
        match event.phase {
            bevy::input::touch::TouchPhase::Started => {
                if drag_state.active_touch_id.is_none() {
                    // First finger primary drag touch for panning
                    drag_state.is_dragging = true;
                    drag_state.drag_start = Some(event.position);
                    drag_state.current_pos = event.position;
                    drag_state.previous_pos = Some(event.position);
                    drag_state.drag_distance = 0.0;
                    drag_state.active_touch_id = Some(event.id);
                } else if drag_state.secondary_touch_id.is_none()
                    && Some(event.id) != drag_state.active_touch_id
                {
                    // Second finger down start pinch-to-zoom tracking for zoom in/out behavior
                    drag_state.secondary_touch_id = Some(event.id);
                    drag_state.secondary_touch_pos = Some(event.position);
                    // Seed the distance so the first frame delta is zero.
                    drag_state.previous_pinch_distance =
                        Some(drag_state.current_pos.distance(event.position));
                }
            }
            bevy::input::touch::TouchPhase::Moved => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.current_pos = event.position;

                    if let Some(start) = drag_state.drag_start {
                        drag_state.drag_distance = start.distance(drag_state.current_pos);
                    }
                } else if Some(event.id) == drag_state.secondary_touch_id {
                    drag_state.secondary_touch_pos = Some(event.position);
                }
            }
            bevy::input::touch::TouchPhase::Ended | bevy::input::touch::TouchPhase::Canceled => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.is_dragging = false;
                    drag_state.drag_start = None;
                    drag_state.previous_pos = None;
                    drag_state.active_touch_id = None;
                    // If the primary finger lifts, promote secondary to primary touch
                    if let Some(sec_id) = drag_state.secondary_touch_id {
                        drag_state.active_touch_id = Some(sec_id);
                        drag_state.current_pos = drag_state.secondary_touch_pos.unwrap_or_default();
                        drag_state.previous_pos = drag_state.secondary_touch_pos;
                        drag_state.drag_start = drag_state.secondary_touch_pos;
                        drag_state.secondary_touch_id = None;
                        drag_state.secondary_touch_pos = None;
                        drag_state.previous_pinch_distance = None;
                        drag_state.is_dragging = true;
                    }
                } else if Some(event.id) == drag_state.secondary_touch_id {
                    // Secondary finger lifted means we stop zooming and will now start panning
                    drag_state.secondary_touch_id = None;
                    drag_state.secondary_touch_pos = None;
                    drag_state.previous_pinch_distance = None;
                }
            }
        }
    }
}

/// Free-look camera system - rotates camera based on mouse or touch drag
fn free_look_camera(
    mouse: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<CursorMoved>,
    mut touch_events: MessageReader<TouchInput>,
    mut drag_state: ResMut<DragState>,
    time: Res<Time>,
    mut camera_query: Query<
        (Entity, &mut Transform, &mut FreeLookCamera, &CameraOrbit),
        With<MainCamera>,
    >,
    mut commands: Commands,
) {
    // Only apply free-look if dragging and moved more than threshold I pulled
    // out of my butt aka 5 px
    if !drag_state.is_dragging || drag_state.drag_distance < 5.0 {
        return;
    }

    // Two fingers on screen means a zoom gesture so we stop roatating around
    // the origin anymore.
    if drag_state.secondary_touch_id.is_some() {
        return;
    }

    let Ok((entity, mut transform, mut free_look, orbit)) = camera_query.single_mut() else {
        return;
    };

    // Handle mouse motion first I guess, why not? Minot?
    if mouse.pressed(MouseButton::Left) {
        for event in mouse_motion.read() {
            if let Some(delta) = event.delta {
                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;

                // Clamp pitch to prevent camera flips, its weird and I don't like it
                free_look.pitch = free_look.pitch.clamp(-1.5, 1.5);

                let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
                let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
                let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();

                transform.translation = Vec3::new(x, y, z);
                *transform = transform.looking_at(orbit.center, Vec3::Y);

                drag_state.previous_pos = Some(event.position);
            }
        }
    }

    // Handle touch motion too cause ipads and whatnot I guess
    for event in touch_events.read() {
        if event.phase == bevy::input::touch::TouchPhase::Moved
            && Some(event.id) == drag_state.active_touch_id
        {
            if let Some(prev_pos) = drag_state.previous_pos {
                let delta = event.position - prev_pos;

                // blah blah no auto rotate here too
                commands
                    .entity(entity)
                    .insert(FreeLookActive(time.elapsed_secs_f64()));

                free_look.yaw -= delta.x * free_look.sensitivity;
                free_look.pitch -= delta.y * free_look.sensitivity;

                // NEW COMMENTS FOR NO RAISIN
                free_look.pitch = free_look.pitch.clamp(-1.5, 1.5);

                let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
                let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
                let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();

                transform.translation = Vec3::new(x, y, z);
                *transform = transform.looking_at(orbit.center, Vec3::Y);
            }

            drag_state.previous_pos = Some(event.position);
        }
    }
}

/// Nuke the FreeLookActive marker after 2 seconds and allow rotation again if
/// thats enabled cause y not
fn cleanup_free_look_after_inactivity(
    time: Res<Time>,
    query: Query<(Entity, &FreeLookActive)>,
    mut commands: Commands,
) {
    let current_time = time.elapsed_secs_f64();
    const INACTIVITY_THRESHOLD: f64 = 2.0;

    for (entity, free_look) in query.iter() {
        if current_time - free_look.0 > INACTIVITY_THRESHOLD {
            commands.entity(entity).remove::<FreeLookActive>();
        }
    }
}

/// Zoom the camera in/out.
///
/// Only two ways to zoom are scroll wheel/mouse on desktop and/or pinch to zoom
/// on say mobile/touchscreens. Both seem the sanest options
///
/// **Perspective mode** zoom moves the camera along the orbital sphere by
/// changing `CameraOrbit.radius`, then recomputes the translation from the
/// current yaw/pitch. Nothing special but here cause I forget easy.
///
/// **Orthographic mode** moving the camera does nothing visually in the same
/// way as perspective, so zoom adjusts `OrthographicProjection::scale` instead.
fn zoom_camera(
    mut wheel: MessageReader<MouseWheel>,
    mut drag_state: ResMut<DragState>,
    mut camera_query: Query<
        (
            &mut Transform,
            &mut CameraOrbit,
            &FreeLookCamera,
            &mut Projection,
        ),
        With<MainCamera>,
    >,
    #[cfg(feature = "egui")] egui_wants_input: Res<ui::EguiWantsInput>,
) {
    #[cfg(feature = "egui")]
    if egui_wants_input.wants_pointer {
        return;
    }

    let Ok((mut transform, mut orbit, free_look, mut projection)) = camera_query.single_mut()
    else {
        return;
    };

    // Accumulate a signed zoom delta from inputs.
    // Positive is zoom in via (shrink radius / scale), and negative = zoom out.
    let mut zoom_delta: f32 = 0.0;

    // Mouse wheel zoom option
    for event in wheel.read() {
        let scroll = match event.unit {
            MouseScrollUnit::Line => event.y * 0.5,
            MouseScrollUnit::Pixel => event.y * 0.005,
        };
        // Scrolling up (positive y) zooms in
        zoom_delta += scroll;
    }

    // Pinch to zoom block
    if let (Some(sec_pos), Some(prev_dist)) = (
        drag_state.secondary_touch_pos,
        drag_state.previous_pinch_distance,
    ) {
        let current_dist = drag_state.current_pos.distance(sec_pos);
        // Spread fingers apart = zoom in (positive delta)
        // pinch = zoom out or crush peoples heads if you load a head gltf
        // never squash heads.
        zoom_delta += (current_dist - prev_dist) * 0.01;
        drag_state.previous_pinch_distance = Some(current_dist);
    }

    if zoom_delta == 0.0 {
        return;
    }

    match *projection {
        Projection::Perspective(_) => {
            // The orbital radius controls the camera position. Note we clamp to
            // 0 so we can't portal jump through the origin in perspective mode.
            orbit.radius = (orbit.radius - zoom_delta).max(0.0);

            let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
            let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
            let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();

            transform.translation = Vec3::new(x, y, z);
            *transform = transform.looking_at(orbit.center, Vec3::Y);
        }
        Projection::Orthographic(ref mut ortho) => {
            // Adjust the orthographic scale, aka smaller is zoom in.
            // Clamps to zero — negative scale mirrors/inverts the scene.
            ortho.scale = (ortho.scale - zoom_delta).max(0.0);
        }

        // Future two finger move on the planar axis goes here
        _ => {}
    }
}

/// Sync ColorState resource to the background clear color.
/// When `color_state.color` is `None` the background follows the active theme
/// dark/light setup. Cause why not.
fn sync_color_state_to_clear_color(
    color_state: Res<ColorState>,
    ui_config: Res<crate::ui::UiConfig>,
    mut clear_color: ResMut<ClearColor>,
) {
    if color_state.is_changed() || ui_config.is_changed() {
        let resolved = match color_state.color {
            Some(c) => c,
            None => {
                let is_dark = match ui_config.theme {
                    crate::ui::ThemeChoice::Dark => true,
                    crate::ui::ThemeChoice::Light => false,
                    crate::ui::ThemeChoice::Auto => {
                        // TODO: Theres DRY code in these here parts that I can refactor over weekend.
                        match dark_light::detect().unwrap_or(dark_light::Mode::Unspecified) {
                            dark_light::Mode::Dark => true,
                            dark_light::Mode::Light | dark_light::Mode::Unspecified => false,
                        }
                    }
                };
                if is_dark {
                    Srgba::new(0.0, 0.0, 0.0, 1.0)
                } else {
                    Srgba::new(1.0, 1.0, 1.0, 1.0)
                }
            }
        };
        clear_color.0 = resolved.into();
    }
}

/// Builds out the initial polars dataframe used for the plot graph shader data.
///
/// Initial len is the same as `flan::PLOT_WINDOW_SIZE` and the y values are
/// clamped between `[0,1]` so the shader code is trivial.
// x is for now ignored as the shader just clamps the max plot points into a
// plot stupidly as an initial implementation.
fn initial_plot_df() -> DataFrame {
    let mut rng = rand::rng();
    let mut y = rng.random_range(0.0f32..=1.0f32);

    let ys: Vec<f32> = (0..flan::PLOT_WINDOW_SIZE)
        .map(|_| {
            let step: f32 = rng.random_range(-0.1..=0.1);
            y = (y + step).clamp(0.0, 1.0);
            y
        })
        .collect();

    let height = flan::PLOT_WINDOW_SIZE;
    DataFrame::new(height, vec![Column::new("y".into(), ys)])
        .expect("initial plot DataFrame construction failed")
}

/// Append a new random-walk row to the plot DataFrame and cap it at
/// `PLOT_WINDOW_SIZE` rows so it never grows beyond what the shader actually uses.
fn tick_plot_data(
    mut plot_df: ResMut<flan::PlotDataFrame>,
    mut events: bevy::ecs::message::MessageWriter<flan::PlotDataUpdated>,
) {
    let mut rng = rand::rng();

    let last_y = plot_df
        .df
        .column("y")
        .ok()
        .and_then(|s| s.f32().ok().map(|ca| ca.get(ca.len().saturating_sub(1))))
        .flatten()
        .unwrap_or(0.5);

    let step: f32 = rng.random_range(-0.1..=0.1);
    let next_y = (last_y + step).clamp(0.0, 1.0);

    let new_row = DataFrame::new(1, vec![Column::new("y".into(), vec![next_y])])
        .expect("new plot row construction failed");

    let combined = plot_df
        .df
        .vstack(&new_row)
        .expect("plot DataFrame vstack failed");

    // Trim to the window we actually use so the DataFrame never grows beyond
    // PLOT_WINDOW_SIZE rows regardless of how long the app runs. No need to
    // waste memory. Could probably make this whole dataframe smaller.
    // TODO: mitch figure out ^^^
    let len = combined.height();
    plot_df.df = if len > flan::PLOT_WINDOW_SIZE {
        combined.slice(
            (len - flan::PLOT_WINDOW_SIZE) as i64,
            flan::PLOT_WINDOW_SIZE,
        )
    } else {
        combined
    };

    events.write(flan::PlotDataUpdated);
}

// TODO: A lot of copy paste between this and the uri parser. Future me can fix it!
#[cfg(test)]
mod tests {
    use crate::ui::world_clock::parse_alarm_entry;

    /// Simulates the comma-split loop used by both the native CLI and the WASM
    /// URL parsers: split on ',', trim each entry, skip empty and unparseable
    /// entries; keep valid `(epoch, tz, label)` triples.
    fn parse_alarm_value(value: &str) -> Vec<(i64, String, Option<String>)> {
        value
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                parse_alarm_entry(entry)
            })
            .collect()
    }

    #[test]
    fn single_two_part_entry() {
        let results = parse_alarm_value("America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        let (epoch, tz, label) = &results[0];
        assert_eq!(*epoch, 1775012160);
        assert_eq!(tz, "America/Chicago");
        assert_eq!(*label, None);
    }

    #[test]
    fn single_three_part_entry_with_label() {
        let results = parse_alarm_value("Birthday:America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        let (epoch, tz, label) = &results[0];
        assert_eq!(*epoch, 1775012160);
        assert_eq!(tz, "America/Chicago");
        assert_eq!(*label, Some("Birthday".to_string()));
    }

    #[test]
    fn comma_separated_two_entries() {
        let results = parse_alarm_value("America/Chicago:1775012160,America/New_York:1893456000");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, "America/Chicago");
        assert_eq!(results[1].1, "America/New_York");
        assert_eq!(results[0].2, None);
        assert_eq!(results[1].2, None);
    }

    #[test]
    fn comma_separated_mixed_label_and_no_label() {
        let results =
            parse_alarm_value("Birthday:America/Chicago:1775012160,America/New_York:1893456000");
        assert_eq!(results.len(), 2);
        let (_, tz0, lbl0) = &results[0];
        let (_, tz1, lbl1) = &results[1];
        assert_eq!(tz0, "America/Chicago");
        assert_eq!(*lbl0, Some("Birthday".to_string()));
        assert_eq!(tz1, "America/New_York");
        assert_eq!(*lbl1, None);
    }

    #[test]
    fn whitespace_around_entries_is_trimmed() {
        // The URL percent_decode step may leave spaces be sure we handle those cases.
        let results = parse_alarm_value("  America/Chicago:1775012160  ,  UTC:1893456000  ");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, "America/Chicago");
        assert_eq!(results[1].1, "UTC");
    }

    #[test]
    fn empty_entry_from_leading_comma_is_skipped() {
        // Leading or trailing , producing blank things to parse that should be ignored
        let results = parse_alarm_value(",America/Chicago:1775012160,");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "America/Chicago");
    }

    #[test]
    fn all_whitespace_entry_is_skipped() {
        let results = parse_alarm_value("   ,America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn invalid_entry_is_skipped_valid_ok() {
        let results = parse_alarm_value("notvalid,America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "America/Chicago");
    }

    #[test]
    fn all_invalid_entries_returns_empty() {
        let results = parse_alarm_value("bad,alsoBad,stillBad");
        assert!(results.is_empty());
    }

    #[test]
    fn entirely_empty_value_returns_empty() {
        let results = parse_alarm_value("");
        assert!(results.is_empty());
    }
}
