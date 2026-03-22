use crate::ai::infer::InferenceEngine;
use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled};
use crate::ui::config::UiConfig;
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::data_viewer::{DataViewerState, ShowDataViewer, data_viewer_window};
use crate::ui::recognizer::{InferenceResult, RecognizerState};
use crate::ui::scroll_view::{ActivePost, POSTS};
use crate::ui::world_clock::{ShowWorldClock, WorldClockState, world_clock_window};
use crate::{ColorState, CubeRotation, FpsDisplay, HueAnimation};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

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

/// Marker component to track whether the Recognizer window is open
#[derive(Component)]
pub struct ShowRecognizer;

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
            .insert_non_send_resource(engine)
            .add_systems(Startup, setup_egui);

        #[cfg(not(target_arch = "wasm32"))]
        let app = app.init_resource::<DataViewerState>();

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
    }
}

/// Here for the recognizer, I need a CJK font to display kanji/hiragana/katakana
static NOTO_SANS_JP: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf");

/// Default model config JSON for the default model for now, future will be to
/// be able to pick different models.
static DEFAULT_MODEL_CONFIG: &[u8] = include_bytes!("../assets/models/default/config.json");

/// Default model weights for ^^^
static DEFAULT_MODEL_WEIGHTS: &[u8] = include_bytes!("../assets/models/default/model.mpk");

/// Bump up egui text and register NotoSansJP as a CJK fallback.
///
/// Oneshot system, after insertion noto sans jp is inserted to the end of every
/// fontfamily so that latin/ascii uses default font and other codepoints
/// hopefully get hit with noto sans jp for kanji et al.
fn configure_egui_style(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    *done = true;

    let ctx = contexts.ctx_mut()?;

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
fn settings_ui(
    mut contexts: EguiContexts,
    mut color_state: ResMut<ColorState>,
    mut effects_enabled: ResMut<EffectsEnabled>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    cube_rotation_query: Query<Entity, With<CubeRotation>>,
    hue_animation_query: Query<Entity, With<HueAnimation>>,
    show_egui_query: Query<(), With<ShowEgui>>,
    recognizer_query: Query<Entity, With<ShowRecognizer>>,
    #[cfg(not(target_arch = "wasm32"))] data_viewer_query: Query<Entity, With<ShowDataViewer>>,
    world_clock_query: Query<Entity, With<ShowWorldClock>>,
    mut active_post: ResMut<ActivePost>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    mut commands: Commands,
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

            ui.menu_button("Background", |ui| {
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
                let mut fps_enabled = fps_query.single().is_ok();
                if ui.checkbox(&mut fps_enabled, "FPS Display [F]").changed() {
                    if fps_enabled {
                        commands.spawn(FpsDisplay);
                    } else if let Ok(entity) = fps_query.single() {
                        commands.entity(entity).despawn();
                    }
                }
                let mut cube_rotation_enabled = cube_rotation_query.single().is_ok();
                if ui
                    .checkbox(&mut cube_rotation_enabled, "Cube Rotation [C]")
                    .changed()
                {
                    if cube_rotation_enabled {
                        commands.spawn(CubeRotation);
                    } else if let Ok(entity) = cube_rotation_query.single() {
                        commands.entity(entity).despawn();
                    }
                }
                let mut hue_animation_enabled = hue_animation_query.single().is_ok();
                if ui
                    .checkbox(&mut hue_animation_enabled, "Hue Animation [H]")
                    .changed()
                {
                    if hue_animation_enabled {
                        commands.spawn(HueAnimation);
                    } else if let Ok(entity) = hue_animation_query.single() {
                        commands.entity(entity).despawn();
                    }
                }

                ui.separator();

                ui.label(egui::RichText::new("Background color").strong());
                let mut color32 = egui::Color32::from_rgb(
                    (color_state.color.red * 255.0) as u8,
                    (color_state.color.green * 255.0) as u8,
                    (color_state.color.blue * 255.0) as u8,
                );
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut color32,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    let [r, g, b, _] = color32.to_normalized_gamma_f32();
                    color_state.color = bevy::color::Srgba::rgb(r, g, b);
                }
                if ui.button("Reset to Grey").clicked() {
                    color_state.color = bevy::color::Srgba::gray(0.5);
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

            // Push "About" to the right side of the menu bar for build info and
            // attribution stuff I put off till now. Even though my data
            // detection build pipeline for kanji is sus lets attribute things
            // so everyone knows what I used.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            });

            ui.menu_button("Apps", |ui| {
                let wc_open = !world_clock_query.is_empty();
                if ui.selectable_label(wc_open, "World Clock").clicked() {
                    if wc_open {
                        if let Ok(entity) = world_clock_query.single() {
                            commands.entity(entity).despawn();
                        }
                    } else {
                        commands.spawn(ShowWorldClock);
                    }
                    ui.close();
                }

                ui.separator();
                ui.label(egui::RichText::new("Abominable Intelligence").strong());

                if ui.button("Recognizer").clicked() {
                    if recognizer_query.is_empty() {
                        commands.spawn(ShowRecognizer);
                    } else if let Ok(entity) = recognizer_query.single() {
                        commands.entity(entity).despawn();
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
                // moving the window doesn't shift previously drawn strokes.
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

                let stroke = egui::Stroke::new(2.0, egui::Color32::BLACK);

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

    let canvas = InferenceEngine::rasterize(state);
    let matches = engine.run(&canvas);
    *result = InferenceResult {
        canvas: Some(canvas),
        matches,
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
