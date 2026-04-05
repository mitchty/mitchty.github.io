mod ai;
mod assets;
mod fullscreen_effect;
mod post_process;
mod ui;

#[cfg(feature = "feathers")]
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::prelude::*;
use bevy_pretty_text::prelude::*;

/// Resource to hold the current background color state
#[derive(Resource)]
pub struct ColorState {
    pub color: Srgba,
}

// use bevy_old_tv_shader::prelude::*;
use polars::prelude::*;
use rand::RngExt;

use assets::{AssetConfigPlugin, asset_path};
use bevy_fontmesh::prelude::*;
use fullscreen_effect::{
    CameraConfig, CameraOrbit, manage_effect_settings, next_effect, previous_effect, spawn_camera,
    toggle_fullscreen_effect, update_effect_time,
};
use post_process::PostProcessPlugin;
use shaders::ShadersPlugin;
use ui::{ScrollViewPlugin, SettingsUiPlugin, send_scroll_events};

/// Absolute rotation speed
const SPEED: f32 = 2.25;
/// Minimum rotation speed in radians per second
const MIN_SPEED: f32 = -SPEED;
/// Maximum rotation speed in radians per second
const MAX_SPEED: f32 = SPEED;
/// Golden angle for rotation calculations
const GOLDEN_ANGLE: f32 = 137.507_77;

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
        .add_plugins(ShadersPlugin)
        .add_plugins(FontMeshPlugin::<StandardMaterial>::default())
        .add_plugins(PostProcessPlugin)
        .add_plugins(PrettyTextPlugin)
        .add_plugins(flan::PlotPlugin)
        .insert_resource(flan::PlotDataFrame {
            df: initial_plot_df(),
        })
        .insert_resource(ClearColor(Color::srgb(0.5, 0.5, 0.5)))
        .insert_resource(ColorState {
            color: Srgba::gray(0.5),
        })
        .insert_resource(ui_config)
        .init_resource::<CameraConfig>()
        .init_resource::<Text3dContent>();

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
        .add_systems(Startup, (setup, setup_fps_ui, setup_3d_text, spawn_camera))
        .add_systems(
            Update,
            tick_plot_data.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_millis(100),
            )),
        )
        .add_systems(
            Update,
            (
                sync_color_state_to_clear_color,
                track_input_drag,
                free_look_camera,
                cleanup_free_look_after_inactivity,
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
                update_fps_display.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs_f32(0.5),
                )),
                // Using a tuple here keeps the overall tuple under Bevy's 20-item
                // limit need to think about how to do all these system setups
                // better. That is "a future mitch problem"
                // TODO: Future sucker me figure it out or brain on it in bg.
                sync_text3d_to_active_post.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
            ),
        );

    // Touch help overlay
    app.add_systems(Startup, setup_help_text)
        .add_systems(Update, dismiss_help_on_input);

    app.run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // let diffuse_path = asset_path("environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2");
    // let specular_path = asset_path("environment_maps/pisa_specular_rgb9e5_zstd.ktx2");

    let cube = meshes.add(Cuboid::new(0.5, 0.5, 0.5));

    let mut hsla = Hsla::hsl(0.0, 1.0, 0.5);
    let mut rng = rand::rng();

    for x in -1..2 {
        for z in -1..2 {
            let base_speed = Vec3::new(
                rng.random_range(MIN_SPEED..=MAX_SPEED),
                rng.random_range(MIN_SPEED..=MAX_SPEED),
                rng.random_range(MIN_SPEED..=MAX_SPEED),
            );

            commands.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(materials.add(Color::from(hsla))),
                Transform::from_translation(Vec3::new(x as f32, 0.0, z as f32)),
                Rotator { base_speed },
                CubeRotationEnabled,
                HueAnimationEnabled,
            ));
            hsla = hsla.rotate_hue(GOLDEN_ANGLE);
        }
    }
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

/// System to update fps display when toggled
fn update_fps_display(
    time: Res<Time>,
    mut fps_text_query: Query<&mut Text, With<FpsText>>,
    fps_display_query: Query<(), With<FpsDisplay>>,
) {
    if fps_display_query.is_empty() {
        // Cheat, no text when the markers off
        for mut text in fps_text_query.iter_mut() {
            if !text.0.is_empty() {
                text.0.clear();
            }
        }
    } else {
        let fps = 1.0 / time.delta_secs();
        for mut text in fps_text_query.iter_mut() {
            text.0 = format!("{:.1} fps", fps);
        }
    }
}

// TODO: port over the tv effect shader or try making my own with the full
// screen shaders now in 0.18? MORE FUTURE MITCH WORK!
// /// Toggle TV effect on the main camera
// /// t toggles on/off
// fn toggle_tv_effect(
//     keyboard: Res<ButtonInput<KeyCode>>,
//     tv_effect_query: Query<Entity, With<TvEffectEnabled>>,
//     mut commands: Commands,
// ) {
//     if keyboard.just_pressed(KeyCode::KeyT) {
//         if let Ok(entity) = tv_effect_query.single() {
//             commands.entity(entity).despawn();
//         } else {
//             commands.spawn(TvEffectEnabled);
//         }
//     }
// }

// /// Apply or remove TV effect toggle
// fn apply_tv_effect(
//     tv_effect_query: Query<(), With<TvEffectEnabled>>,
//     camera_query: Query<(Entity, Has<OldTvSettings>), With<MainCamera>>,
//     tv_settings: Res<TvSettingsResource>,
//     mut commands: Commands,
// ) {
//     let tv_should_be_enabled = !tv_effect_query.is_empty();

//     for (entity, has_tv_settings) in camera_query.iter() {
//         if tv_should_be_enabled && !has_tv_settings {
//             commands.entity(entity).insert(tv_settings.settings);
//         } else if !tv_should_be_enabled && has_tv_settings {
//             commands.entity(entity).remove::<OldTvSettings>();
//         }
//     }
// }

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
                // Only track first touch
                if drag_state.active_touch_id.is_none() {
                    drag_state.is_dragging = true;
                    drag_state.drag_start = Some(event.position);
                    drag_state.current_pos = event.position;
                    drag_state.previous_pos = Some(event.position);
                    drag_state.drag_distance = 0.0;
                    drag_state.active_touch_id = Some(event.id);
                }
            }
            bevy::input::touch::TouchPhase::Moved => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.current_pos = event.position;

                    if let Some(start) = drag_state.drag_start {
                        drag_state.drag_distance = start.distance(drag_state.current_pos);
                    }
                }
            }
            bevy::input::touch::TouchPhase::Ended | bevy::input::touch::TouchPhase::Canceled => {
                if Some(event.id) == drag_state.active_touch_id {
                    drag_state.is_dragging = false;
                    drag_state.drag_start = None;
                    drag_state.previous_pos = None;
                    drag_state.active_touch_id = None;
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

/// Sync ColorState resource the background clear color
fn sync_color_state_to_clear_color(
    color_state: Res<ColorState>,
    mut clear_color: ResMut<ClearColor>,
) {
    if color_state.is_changed() {
        clear_color.0 = color_state.color.into();
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

/// Append a new random-walk row to the underlying plot DataFrame. Then notify
/// `flan` vi a message event to resync the shader data.
///
/// Only at most `flan::PLOT_WINDOW_SIZE` pieces of data are ever yeeted to the
/// plot shader.
fn tick_plot_data(
    mut plot_df: ResMut<flan::PlotDataFrame>,
    mut events: bevy::ecs::message::MessageWriter<flan::PlotDataUpdated>,
) {
    let mut rng = rand::rng();

    // Grab the most recent Y so the random walk doesn't spike weirdly for now.
    // TODO: Maybe have this be exponentially weighted against the edges of the plot?
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

    plot_df.df = plot_df
        .df
        .vstack(&new_row)
        .expect("plot DataFrame vstack append failed");

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
