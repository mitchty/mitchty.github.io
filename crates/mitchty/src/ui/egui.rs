use crate::ai::infer::InferenceEngine;
use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled};
use crate::ui::config::{ThemeChoice, UiConfig};
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::data_viewer::{DataViewerState, ShowDataViewer, data_viewer_window};
#[cfg(debug_assertions)]
use crate::ui::recognizer::RasterSize;
use crate::ui::recognizer::{BASE_BRUSH_R, InferenceResult, RecognizerState};
use crate::ui::scroll_view::{ActivePost, POSTS};
use crate::ui::world_clock::{ShowWorldClock, WorldClockState, world_clock_window};
use crate::{CameraMode, ColorState, CubeRotation, FpsDisplay, HueAnimation, MainCamera};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

// Wasm js bridge types for dark/light changes.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;

/// Resource to track if egui is currently using input, helps with accidental
/// clicks not bleeding downwards to bevy.
#[derive(Resource, Default)]
pub struct EguiWantsInput {
    pub wants_pointer: bool,
    pub wants_keyboard: bool,
}

/// Marker component for the egui ui display
#[derive(Component)]
pub struct ShowEgui;

/// Message for when camera projection needs to change.
///
/// Bound to the M key and for now the egui menu item.
/// `apply_camera_projection_toggle` handles the actual Projection swap on the
/// camera.
#[derive(Debug, Clone, Copy)]
pub struct ToggleCameraProjection;
impl bevy::ecs::message::Message for ToggleCameraProjection {}

/// Marker...ish component to note we were tasked with toggling the camera
/// projection.
#[derive(Resource, Default)]
pub struct CameraProjectionToggleRequested(pub bool);

/// Marker component to track whether the Recognizer window is open
#[derive(Component)]
pub struct ShowRecognizer;

/// Bevy message for when the outside theme has changed.
///
/// For native, a 1 second poll system fires the event on chage.
/// For wasm `matchMedia` listener used by the `EguiPrimaryContextPass`
#[derive(Debug, Clone, Copy)]
pub struct ThemeChanged(pub dark_light::Mode);
impl bevy::ecs::message::Message for ThemeChanged {}

/// Resource for the last OS/browser theme seen.
///
/// Native only, wasm uses the listener directly.
///
/// Pre pump the value so first poll works. Defaults to `Mode::Default` if
/// detection fails.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct LastKnownTheme(pub dark_light::Mode);

#[cfg(not(target_arch = "wasm32"))]
impl Default for LastKnownTheme {
    fn default() -> Self {
        Self(dark_light::detect())
    }
}

/// Here to keep `matchMedia` closure and `MediaQueryList` alive for the apps
/// lifetime otherwise we can't get data from the listeners as they unregister.
#[cfg(target_arch = "wasm32")]
pub struct WasmThemeListener {
    /// The wasm_bindgen closure listener
    _closure: Closure<dyn FnMut(web_sys::MediaQueryListEvent)>,
    /// The MediaQueryList listener
    _mql: web_sys::MediaQueryList,
}

/// Plugin for egui UI
pub struct SettingsUiPlugin;

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        // Try loading the default model from embedded bytes first. This
        // always works on WASM and guarantees a model is available on native
        // even if running outside the repo tree.
        //
        // TODO: Not sure if I want to continue this approach but its for
        // simplicity right now.
        let engine = InferenceEngine::from_embedded(DEFAULT_MODEL_CONFIG, DEFAULT_MODEL_WEIGHTS);

        // On native builds, fall back to the on-disk artifact directories so
        // that a freshly-trained model in recognizer/ still overrides the
        // compiled-in default during development. We do this for debug builds.
        // Note: explicitly exclude wasm32 InferenceEngine::load is native-only for now.
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        let engine = engine.or_else(|| {
            ["recognizer", "../../recognizer", "../recognizer"]
                .iter()
                .find_map(|dir| InferenceEngine::load(dir))
        });

        if engine.is_none() {
            bevy::log::warn!("InferenceEngine: no model loaded; inference will be disabled");
        }

        let app = app
            .init_resource::<EguiWantsInput>()
            .init_resource::<RecognizerState>()
            .init_resource::<InferenceResult>()
            .add_message::<ThemeChanged>()
            .add_message::<ToggleCameraProjection>()
            .init_resource::<CameraProjectionToggleRequested>()
            .insert_non_send_resource(engine)
            .add_systems(Startup, setup_egui)
            .add_systems(
                Update,
                (
                    send_camera_projection_toggle,
                    apply_camera_projection_toggle,
                )
                    .chain(),
            );

        #[cfg(not(target_arch = "wasm32"))]
        let app = app
            .init_resource::<LastKnownTheme>()
            .init_resource::<DataViewerState>()
            .add_systems(
                Update,
                poll_system_theme.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
            );

        // WASM: register the matchMedia listener at startup and drain its
        // pending changes every frame.
        #[cfg(target_arch = "wasm32")]
        let app = app
            .add_systems(Startup, setup_wasm_theme_listener)
            .add_systems(Update, drain_wasm_theme_events);

        app.add_systems(
            EguiPrimaryContextPass,
            (
                configure_egui_style,
                settings_ui,
                #[cfg(not(target_arch = "wasm32"))]
                (recognizer_window, data_viewer_window, world_clock_window).chain(),
                #[cfg(target_arch = "wasm32")]
                (recognizer_window, world_clock_window).chain(),
                update_egui_input_state,
            )
                .chain(),
        );

        // apply_theme_change consumes ThemeChanged messages and updates egui
        // visuals. Runs in the same pass but separately to avoid poisoning the
        // chained tuple above with its different system signature.
        app.add_systems(
            EguiPrimaryContextPass,
            apply_theme_change.after(configure_egui_style),
        );
    }
}

/// Here for the recognizer, I need a CJK font to display kanji/hiragana/katakana
static NOTO_SANS_JP: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf");

/// Default model config JSON for the default model for now, future will be to
/// be able to pick different models.
static DEFAULT_MODEL_CONFIG: &[u8] = include_bytes!("../assets/models/default/config.json");

/// Default model weights for ^^^
static DEFAULT_MODEL_WEIGHTS: &[u8] = include_bytes!("../assets/models/default/model.mpk");

/// Native system poll `dark_light::detect()` once per second.
///
/// Emits `ThemeChanged` when the detected mode actually differs from the
/// last seen value and the user didn't pick one manually.
#[cfg(not(target_arch = "wasm32"))]
fn poll_system_theme(
    ui_config: Res<UiConfig>,
    mut last: ResMut<LastKnownTheme>,
    mut events: bevy::ecs::message::MessageWriter<ThemeChanged>,
) {
    // User choice wins always. Nothing to do.
    if ui_config.theme != ThemeChoice::Auto {
        return;
    }

    let current = dark_light::detect();
    if current != last.0 {
        last.0 = current;
        events.write(ThemeChanged(current));
    }
}

/// WASM system to register a `matchMedia` `change` listener at startup.
///
/// The listener writes the new mode into `WasmThemeListener::pending`. A
/// separate per-frame system drains values into the Bevy event bus.
/// Both the `Closure` and `MediaQueryList` listeners need to always be alive, hence the
/// `NonSendMut` resource that owns them for the app's duration.
#[cfg(target_arch = "wasm32")]
fn setup_wasm_theme_listener(world: &mut World) {
    use std::sync::{Arc, Mutex};

    // Shared slot read by the drain system from the js closure interop.
    let pending: Arc<Mutex<Option<dark_light::Mode>>> = Arc::new(Mutex::new(None));
    let pending_clone = pending.clone();

    // TODO: I have no idea wat to do on failures here. I barely understand wasm
    // as it is js interop is veritable black magique.
    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            bevy::log::warn!("setup_wasm_theme_listener: no window object, skipping");
            return;
        }
    };

    let mql = match window.match_media("(prefers-color-scheme: dark)") {
        Ok(Some(mql)) => mql,
        _ => {
            bevy::log::warn!(
                "setup_wasm_theme_listener: matchMedia not available, skipping listener"
            );
            return;
        }
    };

    let closure: Closure<dyn FnMut(web_sys::MediaQueryListEvent)> =
        Closure::wrap(Box::new(move |e: web_sys::MediaQueryListEvent| {
            let mode = if e.matches() {
                dark_light::Mode::Dark
            } else {
                dark_light::Mode::Light
            };
            if let Ok(mut slot) = pending_clone.lock() {
                *slot = Some(mode);
            }
        }));

    if let Err(err) =
        mql.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
    {
        bevy::log::warn!(
            "setup_wasm_theme_listener: failed to add event listener: {:?}",
            err
        );
        return;
    }

    // Insert as a NonSend resource so both closure and mql stay alive for the exec duration
    world.insert_non_send_resource(WasmThemeListener {
        _closure: closure,
        _mql: mql,
    });

    // Store the Arc so the drain can read directly without an unsafe block.
    world.insert_resource(WasmThemePending(pending));
}

/// Mutable Resource slot shared between the JS closure and the drain system.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
struct WasmThemePending(std::sync::Arc<std::sync::Mutex<Option<dark_light::Mode>>>);

/// WASM-only system to drain the shared pending slot Resource Arc into a Bevy
/// `ThemeChanged` event.
#[cfg(target_arch = "wasm32")]
fn drain_wasm_theme_events(
    ui_config: Res<UiConfig>,
    pending_res: Option<Res<WasmThemePending>>,
    mut events: bevy::ecs::message::MessageWriter<ThemeChanged>,
) {
    // User choice wins always. Nothing to do.
    if ui_config.theme != ThemeChoice::Auto {
        return;
    }

    let Some(pending_res) = pending_res else {
        return;
    };

    let Ok(mut slot) = pending_res.0.lock() else {
        return;
    };

    if let Some(mode) = slot.take() {
        events.write(ThemeChanged(mode));
    }
}

/// Apply an incoming `ThemeChanged` event to the egui context visuals.
///
/// Runs in `EguiPrimaryContextPass` so the context is always valid.
/// Skipped entirely when the user has set a `ThemeChoice` value of dark/light.
fn apply_theme_change(
    mut contexts: EguiContexts,
    ui_config: Res<UiConfig>,
    mut events: bevy::ecs::message::MessageReader<ThemeChanged>,
) -> Result {
    // User picked something, drain queue and call it a day.
    if ui_config.theme != ThemeChoice::Auto {
        for _ in events.read() {}
        return Ok(());
    }

    // Consume all pending events; last one wins but only one should be there.
    // Just being safe.
    let mut new_visuals: Option<egui::Visuals> = None;
    for ThemeChanged(mode) in events.read() {
        new_visuals = Some(match mode {
            dark_light::Mode::Dark => egui::Visuals::dark(),
            dark_light::Mode::Light | dark_light::Mode::Default => egui::Visuals::light(),
        });
    }

    if let Some(visuals) = new_visuals {
        let ctx = contexts.ctx_mut()?;
        ctx.set_visuals(visuals);
    }

    Ok(())
}

/// Resolve which egui visuals to apply at startup.
///
/// First to win checks:
///   - User explicit `ThemeChoice::Dark` or `ThemeChoice::Light`
///   - OS or browser preference via `dark-light`
///   - Local time-of-day hack: 07:00–17:59 is Light, otherwise go for Dark
///   - If all that crap failed Dark
fn resolve_initial_theme(choice: ThemeChoice) -> egui::Visuals {
    // User
    match choice {
        ThemeChoice::Dark => return egui::Visuals::dark(),
        ThemeChoice::Light => return egui::Visuals::light(),
        ThemeChoice::Auto => {}
    }

    // dark-light
    match dark_light::detect() {
        dark_light::Mode::Dark => return egui::Visuals::dark(),
        dark_light::Mode::Light => return egui::Visuals::light(),
        // Default = platform has no preference, fall through.
        dark_light::Mode::Default => {}
    }

    // Hold my beer and use time of day to SWAG a guesstimate.
    let hour = jiff::Zoned::now().hour();
    let use_light = (7..18).contains(&hour);

    if use_light {
        return egui::Visuals::light();
    }

    // If we ever get here just go for Dark, its like the Rock of Roshambo for
    // themes.
    egui::Visuals::dark()
}

/// Returns the default background color for a given theme choice.
// TODO: This is a good candidate for a theme kinda system maybe or plugin.
fn theme_default_color(choice: ThemeChoice) -> bevy::color::Srgba {
    let is_dark = match choice {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::Auto => resolve_initial_theme(ThemeChoice::Auto).dark_mode,
    };
    if is_dark {
        bevy::color::Srgba::new(0.0, 0.0, 0.0, 1.0)
    } else {
        bevy::color::Srgba::new(1.0, 1.0, 1.0, 1.0)
    }
}

/// Bump up egui text, register NotoSansJP as a CJK fallback, and apply the
/// resolved startup theme.
///
/// Oneshot system: after insertion noto sans jp is inserted to the end of every
/// fontfamily so that latin/ascii uses the default font and other codepoints
/// hopefully get hit with noto sans jp for kanji et al.
fn configure_egui_style(
    mut contexts: EguiContexts,
    ui_config: Res<UiConfig>,
    mut done: Local<bool>,
) -> Result {
    if *done {
        return Ok(());
    }
    *done = true;

    let ctx = contexts.ctx_mut()?;

    // Apply the startup theme before anything else so there is no flash of the
    // wrong theme while fonts are being set up.
    ctx.set_visuals(resolve_initial_theme(ui_config.theme));

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "NotoSansJP".to_owned(),
        egui::FontData::from_static(NOTO_SANS_JP).into(),
    );

    // Only use noto sans jp for anything that fails to render in a latin subset
    for family_data in fonts.families.values_mut() {
        family_data.push("NotoSansJP".to_owned());
    }
    ctx.set_fonts(fonts);

    ctx.style_mut(|style| {
        for font_id in style.text_styles.values_mut() {
            font_id.size += 4.0;
        }
    });

    Ok(())
}

/// Spawn marker entities for egui state. This is what maps from cli/clap args
/// or wasm query params to setup the bevy ecs so we can provide "links" to
/// certain features/posts etc...
///
/// This just provides a way for the ECS to start from. Once up the ecs is used
/// as normal nothing from here is reused at runtime.
fn setup_egui(
    mut commands: Commands,
    mut effects_enabled: ResMut<EffectsEnabled>,
    mut active_post: ResMut<ActivePost>,
    ui_config: Res<UiConfig>,
) {
    // Start with effects enabled by default
    effects_enabled.0 = true;

    // Always-on markers
    commands.spawn(CubeRotation);
    commands.spawn(HueAnimation);
    commands.spawn(FpsDisplay);

    // Menu bar: on by default, but UiConfig lets callers suppress it... for
    // now.
    if ui_config.show_menu_bar {
        commands.spawn(ShowEgui);
    }

    if ui_config.show_world_clock {
        commands.spawn(ShowWorldClock);
    }

    if ui_config.show_recognizer {
        commands.spawn(ShowRecognizer);
    }

    #[cfg(not(target_arch = "wasm32"))]
    if ui_config.show_data_viewer {
        commands.spawn(ShowDataViewer);
    }

    if let Some(idx) = ui_config.initial_post {
        *active_post = ActivePost(Some(idx));
    }

    // Build WorldClockState from UiConfig overrides. insert_resource replaces
    // the defaults for startup if provided.
    let wc_state = WorldClockState::from_config(
        &ui_config.initial_timezones,
        &ui_config.initial_alarms,
        ui_config.initial_sort_col,
        ui_config.initial_sort_dir,
        ui_config.initial_pinned,
    );
    commands.insert_resource(wc_state);
}

/// Display the settings UI using egui as a top menu bar
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn settings_ui(
    mut contexts: EguiContexts,
    mut color_state: ResMut<ColorState>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    show_egui_query: Query<(), With<ShowEgui>>,
    // In ParamSet now to avoid 16 param system tuple limit in bevy.
    mut marker_queries: ParamSet<(
        Query<Entity, With<FpsDisplay>>,
        Query<Entity, With<CubeRotation>>,
        Query<Entity, With<HueAnimation>>,
        Query<Entity, With<ShowRecognizer>>,
        Query<Entity, With<ShowWorldClock>>,
        Query<&mut Visibility, With<flan::PlotUiNode>>,
    )>,
    #[cfg(not(target_arch = "wasm32"))] data_viewer_query: Query<Entity, With<ShowDataViewer>>,
    mut active_post: ResMut<ActivePost>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    mut commands: Commands,
    mut ui_config: ResMut<UiConfig>,
    mut proj_params: ParamSet<(Res<CameraMode>, ResMut<CameraProjectionToggleRequested>)>,
) -> Result {
    if show_egui_query.is_empty() {
        return Ok(());
    }

    trace!("settings_ui running - ShowEgui exists");

    egui::TopBottomPanel::top("menu_bar").show(contexts.ctx_mut()?, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            // File menu, for now its just for a quit menu item on non wasm targets
            #[cfg(not(target_arch = "wasm32"))]
            ui.menu_button("File", |ui| {
                if ui.button("Quit").clicked() {
                    std::process::exit(0);
                }
            });

            ui.menu_button("Gooey", |ui| {
                ui.label(egui::RichText::new("Projection").strong());
                let proj_label = match *proj_params.p0() {
                    CameraMode::Perspective => "Perspective [M]",
                    CameraMode::Orthographic => "Orthographic [M]",
                };
                if ui.button(proj_label).clicked() {
                    proj_params.p1().0 = true;
                }

                ui.separator();

                ui.label(egui::RichText::new("Effects").strong());
                let mut fullscreen_enabled = effects_enabled.0;
                if ui
                    .checkbox(&mut fullscreen_enabled, "Fullscreen Effect [E]")
                    .changed()
                {
                    effects_enabled.0 = fullscreen_enabled;
                }
                ui.label("Shader:");
                for (idx, shader_info) in available_shaders.shaders.iter().enumerate() {
                    let is_selected = active_shader.index == idx;
                    if ui
                        .selectable_label(is_selected, &shader_info.display_name)
                        .clicked()
                    {
                        active_shader.index = idx;
                        trace!(
                            "shader effect changed to {}",
                            active_shader.display_name(&available_shaders)
                        );
                    }
                }

                ui.separator();

                ui.label(egui::RichText::new("Toggles").strong());
                let fps_entity = marker_queries.p0().single().ok();
                let mut fps_enabled = fps_entity.is_some();
                if ui.checkbox(&mut fps_enabled, "FPS Display [F]").changed() {
                    if fps_enabled {
                        commands.spawn(FpsDisplay);
                    } else if let Some(entity) = fps_entity {
                        commands.entity(entity).despawn();
                    }
                }
                let cube_entity = marker_queries.p1().single().ok();
                let mut cube_rotation_enabled = cube_entity.is_some();
                if ui
                    .checkbox(&mut cube_rotation_enabled, "Cube Rotation [C]")
                    .changed()
                {
                    if cube_rotation_enabled {
                        commands.spawn(CubeRotation);
                    } else if let Some(entity) = cube_entity {
                        commands.entity(entity).despawn();
                    }
                }
                let hue_entity = marker_queries.p2().single().ok();
                let mut hue_animation_enabled = hue_entity.is_some();
                if ui
                    .checkbox(&mut hue_animation_enabled, "Hue Animation [H]")
                    .changed()
                {
                    if hue_animation_enabled {
                        commands.spawn(HueAnimation);
                    } else if let Some(entity) = hue_entity {
                        commands.entity(entity).despawn();
                    }
                }

                ui.separator();

                ui.label(egui::RichText::new("Background color").strong());
                // Resolve the display color: user pick or theme default.
                let display = color_state
                    .color
                    .unwrap_or_else(|| theme_default_color(ui_config.theme));
                let mut color32 = egui::Color32::from_rgb(
                    (display.red * 255.0) as u8,
                    (display.green * 255.0) as u8,
                    (display.blue * 255.0) as u8,
                );
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut color32,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    let [r, g, b, _] = color32.to_normalized_gamma_f32();
                    color_state.color = Some(bevy::color::Srgba::rgb(r, g, b));
                }
                // Only show reset button when the user has overridden the theme default.
                if color_state.color.is_some() && ui.button("Reset to default").clicked() {
                    color_state.color = None;
                    ui.close();
                }
            });

            ui.menu_button("Posts", |ui| {
                for (idx, post) in POSTS.iter().enumerate() {
                    let is_active = active_post.0 == Some(idx);
                    if ui.selectable_label(is_active, post.name).clicked() {
                        active_post.0 = if is_active { None } else { Some(idx) };
                        ui.close();
                    }
                }
            });

            ui.menu_button("Apps", |ui| {
                let wc_entity = marker_queries.p4().single().ok();
                let wc_open = wc_entity.is_some();
                if ui.selectable_label(wc_open, "World Clock").clicked() {
                    if let Some(entity) = wc_entity {
                        commands.entity(entity).despawn();
                    } else {
                        commands.spawn(ShowWorldClock);
                    }
                    ui.close();
                }
            });

            // About and Experiments ar on the RHS of the menu bar. NOte due to
            // the ordering stuff to the left aka Experiments goes after the
            // About definition.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 3-way theme toggle, Auto means heuristic pick for me,
                // Dark/Light is user pref
                let (label, next) = match ui_config.theme {
                    ThemeChoice::Auto => ("🌓", ThemeChoice::Dark),
                    ThemeChoice::Dark => ("🌙", ThemeChoice::Light),
                    ThemeChoice::Light => ("☀", ThemeChoice::Auto),
                };
                if ui.button(label).clicked() {
                    ui_config.theme = next;
                    // Apply the new visuals immediately if clicked.
                    let visuals = match next {
                        ThemeChoice::Dark => egui::Visuals::dark(),
                        ThemeChoice::Light => egui::Visuals::light(),
                        ThemeChoice::Auto => resolve_initial_theme(ThemeChoice::Auto),
                    };
                    ui.ctx().set_visuals(visuals);
                }
                ui.menu_button("About", |ui| {
                    ui.hyperlink_to("GitHub Repo", lib::build_info::GIT_REPO);
                    ui.separator();
                    ui.label(format!("Version:  {}", env!("CARGO_PKG_VERSION")));
                    if lib::build_info::GIT_DIRTY {
                        ui.label(format!(
                            "Commit:   {} modified", // This is what I'll see 99% of the time
                            lib::build_info::GIT_COMMIT
                        ));
                    } else {
                        // iff we're on a build coming from nix the link will be to the commit
                        ui.hyperlink_to(
                            format!("Commit:   {}", lib::build_info::GIT_COMMIT),
                            format!(
                                "{}/commit/{}",
                                lib::build_info::GIT_REPO,
                                lib::build_info::GIT_COMMIT
                            ),
                        );
                    }
                    ui.label(format!("Profile:  {}", lib::build_info::BUILD_PROFILE));
                    ui.label(format!("Rustc:    {}", lib::build_info::RUSTC_VERSION));
                    // TODO: Gate the build date to debug builds only?
                    ui.label(format!("Built:    {}", lib::build_info::BUILD_DATE));
                    ui.separator();
                    ui.label("Third Party Acknowlegements");
                    ui.separator();
                    ui.label("Kanjivg");
                    ui.hyperlink("https://kanjivg.tagaini.net");
                });

                ui.menu_button("Experiments", |ui| {
                    ui.label(egui::RichText::new("Abominable Intelligence").strong());
                    ui.separator();

                    let recognizer_entity = marker_queries.p3().single().ok();
                    if ui.button("Recognizer").clicked() {
                        if let Some(entity) = recognizer_entity {
                            commands.entity(entity).despawn();
                        } else {
                            commands.spawn(ShowRecognizer);
                        }
                        ui.close();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("Data Viewer").clicked() {
                        if data_viewer_query.is_empty() {
                            commands.spawn(ShowDataViewer);
                        } else if let Ok(entity) = data_viewer_query.single() {
                            commands.entity(entity).despawn();
                        }
                        ui.close();
                    }

                    ui.separator();

                    ui.label(egui::RichText::new("Flan Shaders").strong());

                    let line_graph_visible = marker_queries
                        .p5()
                        .single()
                        .map(|v| *v != Visibility::Hidden)
                        .unwrap_or(false);
                    if ui
                        .selectable_label(line_graph_visible, "Line Graph")
                        .clicked()
                    {
                        if let Ok(mut vis) = marker_queries.p5().single_mut() {
                            *vis = if line_graph_visible {
                                Visibility::Hidden
                            } else {
                                Visibility::Visible
                            };
                        }
                        ui.close();
                    }
                });
            });
        });
    });
    Ok(())
}

/// System to render the Recognizer egui window when ShowRecognizer is present.
fn recognizer_window(
    mut contexts: EguiContexts,
    recognizer_query: Query<Entity, With<ShowRecognizer>>,
    mut state: ResMut<RecognizerState>,
    engine: NonSend<Option<InferenceEngine>>,
    mut inference_result: ResMut<InferenceResult>,
    mut commands: Commands,
) -> Result {
    if recognizer_query.is_empty() {
        return Ok(());
    }

    let mut open = true;
    egui::Window::new("Recognizer")
        .open(&mut open)
        .default_size([540.0, 380.0])
        .resizable(true)
        .show(contexts.ctx_mut()?, |ui| {
            ui.horizontal(|ui| {
                let can_undo = !state.strokes.is_empty() || state.current_stroke.is_some();
                let can_redo = !state.redo_stack.is_empty();
                let can_clear = can_undo;

                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    state.undo();
                    // Re-run inference after undo so the sidebar stays current.
                    run_inference(&state, &engine, &mut inference_result);
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    state.redo();
                    run_inference(&state, &engine, &mut inference_result);
                }
                ui.separator();
                if ui
                    .add_enabled(can_clear, egui::Button::new("Clear"))
                    .clicked()
                {
                    state.clear();
                    // Clear inference result alongside the canvas.
                    *inference_result = InferenceResult::default();
                }

                ui.separator();
                // Stroke width changes will re-run classification inference on
                // change. Seems to have helped me realize my training needs
                // more stroke variability.
                ui.label("Stroke:");
                let before_scale = state.stroke_scale;
                ui.add(
                    egui::Slider::new(&mut state.stroke_scale, 0.5_f32..=4.0)
                        .step_by(0.1)
                        .fixed_decimals(1)
                        .suffix("x"),
                )
                .on_hover_text(format!(
                    "Brush radius: {:.2} grid px = (base {BASE_BRUSH_R} x scale x grid/28)",
                    BASE_BRUSH_R * state.raster_size.pixels() as f32 / 28.0 * state.stroke_scale,
                ));
                if (state.stroke_scale - before_scale).abs() > f32::EPSILON
                    && !state.strokes.is_empty()
                {
                    run_inference(&state, &engine, &mut inference_result);
                }

                ui.separator();
                ui.checkbox(&mut state.debug_bbox, "Debug bbox")
                    .on_hover_text(
                        "Draw a red bounding box around the tight crop sent to the classifier",
                    );

                // Size picker: debug builds only, here to let me test models
                // trained at different input resolutions without recompiling.
                // Will rip this out eventually.
                #[cfg(debug_assertions)]
                {
                    ui.separator();
                    ui.label("Size:");
                    let before = state.raster_size;
                    egui::ComboBox::from_id_salt("raster_size")
                        .selected_text(state.raster_size.label())
                        .show_ui(ui, |ui| {
                            //                            for &size in &[RasterSize::S128] { for later...
                            {
                                let &size = &RasterSize::S128;
                                ui.selectable_value(&mut state.raster_size, size, size.label());
                            }
                        });
                    if state.raster_size != before {
                        state.clear();
                        *inference_result = InferenceResult::default();
                    }
                }
            });

            ui.separator();

            const CANVAS_W: f32 = 360.0;
            const CANVAS_H: f32 = 300.0;
            const SIDEBAR_W: f32 = 148.0;
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                let canvas_size = egui::vec2(CANVAS_W, CANVAS_H);
                let (response, painter) =
                    ui.allocate_painter(canvas_size, egui::Sense::click_and_drag());

                painter.rect_filled(response.rect, 4.0, egui::Color32::WHITE);

                // Origin is the top-left of the canvas in screen space.
                // All stroke points are stored relative to this origin so that
                // moving the window doesn't shift previously drawn strokes,
                // that was a fun bug.
                let origin = response.rect.min;

                let pointer_pos = response.interact_pointer_pos();

                // Track whether a stroke was just committed this frame so we
                // can trigger inference exactly once per lift.
                let had_stroke_before = state.current_stroke.is_some();

                if response.is_pointer_button_down_on() {
                    if let Some(pos) = pointer_pos {
                        // Clamp to canvas bounds then convert to local coords.
                        let clamped = egui::pos2(
                            pos.x.clamp(response.rect.min.x, response.rect.max.x),
                            pos.y.clamp(response.rect.min.y, response.rect.max.y),
                        );
                        let local = clamped - origin;
                        state
                            .current_stroke
                            .get_or_insert_with(Vec::new)
                            .push(egui::pos2(local.x, local.y));
                    }
                } else if state.current_stroke.is_some() {
                    state.commit_stroke();
                }

                let stroke_committed = had_stroke_before && state.current_stroke.is_none();
                if stroke_committed {
                    run_inference(&state, &engine, &mut inference_result);
                }

                // Visual stroke width tracks the raster brush radius so what
                // you see on the canvas matches what is sent to the classifier.
                // Base 2.0 px at 1x scale for ref.
                let stroke = egui::Stroke::new(2.0 * state.stroke_scale, egui::Color32::BLACK);

                let to_screen = |p: egui::Pos2| p + origin.to_vec2();

                for segment in &state.strokes {
                    for pair in segment.windows(2) {
                        painter.line_segment([to_screen(pair[0]), to_screen(pair[1])], stroke);
                    }
                }

                if let Some(current) = &state.current_stroke {
                    for pair in current.windows(2) {
                        painter.line_segment([to_screen(pair[0]), to_screen(pair[1])], stroke);
                    }
                }

                // TODO: Keep this debug only? Its kinda useful
                //
                // Debug only, draw a red bounding-box overlay showing the crop
                // that was sent to the classifier on the last inference run.
                //
                // If I need to change stuff with what I send to the classifier
                // this helps with eyeballing if what I'm sending makes sense or
                // not.
                if state.debug_bbox
                    && let Some((bx0, by0, bx1, by1)) = inference_result.bbox
                {
                    let bbox_rect = egui::Rect::from_min_max(
                        to_screen(egui::pos2(bx0, by0)),
                        to_screen(egui::pos2(bx1, by1)),
                    );
                    painter.rect_stroke(
                        bbox_rect,
                        0.0,
                        egui::Stroke::new(1.5, egui::Color32::RED),
                        egui::StrokeKind::Outside,
                    );
                }

                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width(SIDEBAR_W);
                    draw_inference_sidebar(ui, &inference_result, engine.as_ref().as_ref());
                });
            });
        });

    if !open && let Ok(entity) = recognizer_query.single() {
        commands.entity(entity).despawn();
    }

    Ok(())
}

/// Run inference on the current canvas state and store the result.
fn run_inference(
    state: &RecognizerState,
    engine: &Option<InferenceEngine>,
    result: &mut InferenceResult,
) {
    let Some(engine) = engine.as_ref() else {
        return;
    };

    let (canvas, bbox) = InferenceEngine::rasterize(state);
    let matches = engine.run(&canvas);
    *result = InferenceResult {
        canvas: Some(canvas),
        matches,
        bbox,
    };
}

/// Render the inference results into the sidebar.
fn draw_inference_sidebar(
    ui: &mut egui::Ui,
    result: &InferenceResult,
    engine: Option<&InferenceEngine>,
) {
    if engine.is_none() {
        ui.label(egui::RichText::new("No model loaded.").weak().italics());
        return;
    }

    if result.matches.is_empty() {
        ui.label(
            egui::RichText::new("Draw something\nto see results.")
                .weak()
                .italics(),
        );
        return;
    }

    let engine = engine.unwrap();

    ui.label(egui::RichText::new("Top matches:").strong());
    ui.add_space(4.0);

    // Show the top 5 results.
    for (class_idx, confidence) in result.matches.iter().take(5) {
        let pct = confidence * 100.0;
        let label = match engine.char_for_class(*class_idx) {
            Some(ch) => format!("{}  {:.1}%", ch, pct),
            None => format!("[{}]  {:.1}%", class_idx, pct),
        };
        ui.horizontal(|ui| {
            ui.label(&label);
        });
        // Small confidence bar.
        let bar_w = 140.0 * confidence;
        let (bar_rect, _) = ui.allocate_exact_size(egui::vec2(140.0, 6.0), egui::Sense::hover());
        if ui.is_rect_visible(bar_rect) {
            let fill = egui::Color32::from_rgb(80, 140, 220);
            let filled = egui::Rect::from_min_size(bar_rect.min, egui::vec2(bar_w, 6.0));
            ui.painter()
                .rect_filled(bar_rect, 2.0, egui::Color32::from_gray(200));
            ui.painter().rect_filled(filled, 2.0, fill);
        }
        ui.add_space(2.0);
    }
}

/// System to update the EguiWantsInput resource based on egui's input state,
/// mostly here just to make sure egui input doesn't pass down to bevy.
fn update_egui_input_state(
    mut contexts: EguiContexts,
    mut egui_wants_input: ResMut<EguiWantsInput>,
    show_egui_query: Query<(), With<ShowEgui>>,
) -> Result {
    if show_egui_query.is_empty() {
        egui_wants_input.wants_pointer = false;
        egui_wants_input.wants_keyboard = false;
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    // Update the resource with current egui input state
    // This includes clicks, drags, and mouse wheel when over the egui panel...
    // I think, might be missing crap here. I don't know how to program gooeys.
    egui_wants_input.wants_pointer = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
    egui_wants_input.wants_keyboard = ctx.wants_keyboard_input();

    Ok(())
}

/// Drains `CameraProjectionToggleRequested`/abuses the true bool into a
/// `ToggleCameraProjection` message so that `apply_camera_projection_toggle`
/// handles both the key and the button through the same code path.
// TODO: Need to come up with a better hack this is ass.
fn send_camera_projection_toggle(
    mut requested: ResMut<CameraProjectionToggleRequested>,
    mut events: MessageWriter<ToggleCameraProjection>,
) {
    if requested.0 {
        requested.0 = false;
        events.write(ToggleCameraProjection);
    }
}

/// Shared handler for camera projection toggling.
///
/// Based on `ToggleCameraProjection` messages swaps the `Projection` component
/// on the `MainCamera` entity in-place.
pub fn apply_camera_projection_toggle(
    mut events: MessageReader<ToggleCameraProjection>,
    mut camera_mode: ResMut<CameraMode>,
    mut projection_query: Query<&mut Projection, With<MainCamera>>,
) {
    for _ in events.read() {
        let Ok(mut projection) = projection_query.single_mut() else {
            continue;
        };
        let new_mode = match *camera_mode {
            CameraMode::Perspective => {
                *projection = Projection::Orthographic(OrthographicProjection {
                    scale: 5.0,
                    near: 0.1,
                    far: 1000.0,
                    scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                        viewport_height: 1.0,
                    },
                    viewport_origin: Vec2::new(0.5, 0.5),
                    ..OrthographicProjection::default_3d()
                });
                CameraMode::Orthographic
            }
            CameraMode::Orthographic => {
                *projection = Projection::Perspective(PerspectiveProjection::default());
                CameraMode::Perspective
            }
        };
        bevy::log::debug!("projection now {:?}", new_mode);
        *camera_mode = new_mode;
    }
}
