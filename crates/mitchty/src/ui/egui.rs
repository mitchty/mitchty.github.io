use crate::ai::infer::InferenceEngine;
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName, ReverieKey};
use crate::post_process::{ActiveShader, AvailableShaders, EffectsEnabled};
use crate::ui::config::{ThemeChoice, UiConfig};
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::data_viewer::{
    DataViewerState, ImageProcessingCache, ShowDataViewer, data_viewer_window,
};
#[cfg(debug_assertions)]
use crate::ui::recognizer::RasterSize;
use crate::ui::recognizer::{BASE_BRUSH_R, InferenceResult, RecognizerState};
use crate::ui::world_clock::{ShowWorldClock, WorldClockState, world_clock_window};
use crate::{
    CameraMode, ColorState, FpsDisplay, HueAnimation, MainCamera, ShowSceneModel, ShowText3d,
    Text3dDefaultPending, Text3dRenderer,
};
use crate::{SceneConfig, SceneTransformConfig, SceneUrlState};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
#[cfg(not(target_arch = "wasm32"))]
use egui_file_dialog::FileDialog;
use transform_gizmo_bevy::prelude::{GizmoMode, GizmoOptions, GizmoOrientation};

use crate::plugins::theme::{
    EguiThemeSet, ThemePlugin, resolve_initial_theme, theme_default_color,
};
use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};

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

/// Non-Send wrapper around `egui_file_dialog::FileDialog`, Resource needs to
/// match Send to work.
#[cfg(not(target_arch = "wasm32"))]
pub struct SceneFileDialog(pub FileDialog);

/// Message for when camera projection needs to change.
///
/// Bound to the M key and for now the egui menu item.
/// `apply_camera_projection_toggle` handles the actual Projection swap on the
/// camera.
#[derive(Debug, Clone, Copy)]
pub struct ToggleCameraProjection;
impl bevy::ecs::message::Message for ToggleCameraProjection {}

/// Message to indicate the camera should be reset.
#[derive(Debug, Clone, Copy)]
pub struct ResetCamera;
impl bevy::ecs::message::Message for ResetCamera {}

/// Marker...ish component to note we were tasked with toggling the camera
/// projection.
#[derive(Resource, Default)]
pub struct CameraProjectionToggleRequested(pub bool);

/// Marker component to track whether the Recognizer window is open
#[derive(Component)]
pub struct ShowRecognizer;

/// Marker component to track whether the Scene Config window is open
#[derive(Component)]
pub struct ShowSceneConfig;

/// `SystemSet` that gates all per-frame systems owned by the `SettingsUiPlugin`.
///
/// Controlled by `PluginEnabled::<SettingsUiPlugin>`. For now this basically
/// kills egui but I need to make this more granular later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct SettingsUiSystems;

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
            .insert_resource(PluginEnabled::<SettingsUiPlugin>::default())
            .configure_sets(
                Update,
                SettingsUiSystems.run_if(run_if_enabled::<SettingsUiPlugin>()),
            )
            .configure_sets(
                EguiPrimaryContextPass,
                SettingsUiSystems.run_if(run_if_enabled::<SettingsUiPlugin>()),
            )
            .add_plugins(ThemePlugin)
            .init_resource::<EguiWantsInput>()
            .init_resource::<RecognizerState>()
            .init_resource::<InferenceResult>()
            .add_message::<ToggleCameraProjection>()
            .add_message::<ResetCamera>()
            .init_resource::<CameraProjectionToggleRequested>()
            .insert_non_send_resource(engine)
            .add_systems(Startup, setup_egui)
            .add_systems(
                Update,
                (
                    send_camera_projection_toggle,
                    apply_camera_projection_toggle,
                    reset_camera,
                )
                    .chain()
                    .in_set(SettingsUiSystems),
            );

        #[cfg(not(target_arch = "wasm32"))]
        let app = app
            .init_resource::<DataViewerState>()
            .init_resource::<ImageProcessingCache>()
            .insert_non_send_resource(SceneFileDialog(
                FileDialog::new()
                    .title("Load Scene GLTF")
                    .add_file_filter_extensions("GLTF / GLB", vec!["gltf", "glb"]),
            ));

        app.add_systems(
            EguiPrimaryContextPass,
            (
                configure_egui_style.in_set(EguiThemeSet),
                settings_ui,
                #[cfg(not(target_arch = "wasm32"))]
                (
                    recognizer_window,
                    data_viewer_window,
                    world_clock_window,
                    scene_config_window,
                )
                    .chain(),
                #[cfg(target_arch = "wasm32")]
                (recognizer_window, world_clock_window, scene_config_window).chain(),
                update_egui_input_state,
            )
                .chain()
                .in_set(SettingsUiSystems),
        );

        if let Some(mut registry) = app.world_mut().get_resource_mut::<PluginRegistry>() {
            registry.register::<SettingsUiPlugin>("Settings UI", true);
        }
    }
}

/// Here for the recognizer, I need a CJK font to display kanji/hiragana/katakana
static NOTO_SANS_JP: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf");

/// Default model config JSON for the default model for now, future will be to
/// be able to pick different models.
static DEFAULT_MODEL_CONFIG: &[u8] = include_bytes!("../assets/models/default/config.json");

/// Default model weights for ^^^
static DEFAULT_MODEL_WEIGHTS: &[u8] = include_bytes!("../assets/models/default/model.mpk");

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
    mut active_reverie: ResMut<ActiveReverie>,
    ui_config: Res<UiConfig>,
    reveries: Query<(Entity, &ReverieKey, &ReverieDisplayName)>,
) {
    // Start with effects enabled by default
    effects_enabled.0 = true;

    // Always-on markers
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

    // Resolve the raw initial_reverie string -> Entity. Reverie entities are
    // spawned in ReveriesPlugin::build() before any startup system runs, so
    // this query is always populated by the time this system runs.
    if let Some(ref name) = ui_config.initial_reverie {
        crate::plugins::reveries::apply_initial_reverie(name, &reveries, &mut active_reverie);
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

/// Draw the "Load Scene URL" popup window.
///
/// Validates that the input starts with `http://` or `https://` before the Load
/// button can be used. Better error handling needs to happen at some point but
/// thats a future thing.
/// Draw the "Load Scene URL" popup window.
///
/// Future mitch note, if we take a `&mut SceneConfig` in this popup for its
/// duration we cause needless messages about changes to it every tick. Which is
/// dum but its a behavior I never knew about with Resources/Components.
///
/// So this now does a 2 phase commit for the string data.
fn draw_scene_url_popup(ctx: &egui::Context, state: &mut SceneUrlState) {
    if !state.open {
        return;
    }

    let mut open = true;
    egui::Window::new("Load Scene URL")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label("Enter a URL to a .glb or .gltf file:");
            ui.add_space(4.0);

            let response = ui.add(
                egui::TextEdit::singleline(&mut state.buf)
                    .hint_text("https://example.com/example.glb")
                    .desired_width(360.0),
            );
            response.request_focus();

            ui.add_space(6.0);

            let url = state.buf.trim().to_string();
            let valid = url.starts_with("http://") || url.starts_with("https://");

            ui.horizontal(|ui| {
                if ui.add_enabled(valid, egui::Button::new("Load")).clicked() {
                    // Just store the url the user entered here.
                    state.confirmed_url = Some(url.clone());
                    state.buf.clear();
                    state.open = false;
                }
                if ui.button("Cancel").clicked() {
                    state.buf.clear();
                    state.open = false;
                }
            });

            if !valid && !state.buf.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("URL must begin with http:// or https://")
                        .small()
                        .color(egui::Color32::from_rgb(220, 80, 80)),
                );
            }
        });

    if !open {
        state.buf.clear();
        state.open = false;
    }
}

/// System that renders the Scene Config left side panel when `ShowSceneConfig`
/// is present.
///
/// The 3D gizmo itself is rendered by `TransformGizmoPlugin` directly into the
/// Bevy scene - no egui matrix math required. This panel only controls
/// `GizmoOptions` (orientation, active modes) and provides a numeric scale
/// input and Reset button as a precision fallback alongside the gizmo.
#[allow(clippy::too_many_arguments)]
fn scene_config_window(
    mut contexts: EguiContexts,
    scene_config_query: Query<Entity, With<ShowSceneConfig>>,
    show_text3d_query: Query<Entity, With<ShowText3d>>,
    show_scene_model_query: Query<Entity, With<ShowSceneModel>>,
    mut scene_transform: ResMut<SceneTransformConfig>,
    mut gizmo_options: ResMut<GizmoOptions>,
    mut text3d_pending: ResMut<Text3dDefaultPending>,
    mut text3d_renderer: ResMut<Text3dRenderer>,
    mut scene_config: ResMut<SceneConfig>,
    mut scene_url_state: ResMut<SceneUrlState>,
    #[cfg(not(target_arch = "wasm32"))] mut scene_file_dialog: NonSendMut<SceneFileDialog>,
    mut commands: Commands,
) -> Result {
    if scene_config_query.is_empty() {
        return Ok(());
    }

    // Drive the file dialog update and pick result every frame, native only obviously.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = contexts.ctx_mut()?;
        scene_file_dialog.0.update(ctx);
        if let Some(path) = scene_file_dialog.0.take_picked() {
            scene_config.custom_scene = Some(path.to_string_lossy().into_owned());
        }
    }

    // URL popup + 2-phase commit logic for the scene load setup.
    draw_scene_url_popup(contexts.ctx_mut()?, &mut scene_url_state);
    if let Some(url) = scene_url_state.confirmed_url.take() {
        scene_config.custom_scene = Some(url);
    }

    egui::SidePanel::left("scene_config_panel")
        .resizable(true)
        .default_width(240.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(egui::RichText::new("GLTF Config").strong());
            ui.horizontal(|ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("📂 Load File").clicked() {
                    scene_file_dialog.0.pick_file();
                }
                if ui.button("🌐 Load URL").clicked() {
                    scene_url_state.open = true;
                }
                let has_custom = scene_config.custom_scene.is_some();
                if ui
                    .add_enabled(has_custom, egui::Button::new("↩ Reset"))
                    .on_disabled_hover_text("This is already loaded, I can't do that Dave!")
                    .clicked()
                {
                    scene_config.custom_scene = None;
                }
            });

            // Toggle gltf model visibility
            let show_model_entity = show_scene_model_query.single().ok();
            let show_model = show_model_entity.is_some();
            if ui.selectable_label(show_model, "👁 Show Model").clicked() {
                if let Some(entity) = show_model_entity {
                    commands.entity(entity).despawn();
                } else {
                    commands.spawn(ShowSceneModel);
                }
            }

            ui.add_space(6.0);
            ui.separator();

            ui.label(egui::RichText::new("Orientation").strong());
            egui::ComboBox::from_id_salt("gizmo_orientation")
                .selected_text(format!("{:?}", gizmo_options.gizmo_orientation))
                .show_ui(ui, |ui| {
                    for o in [GizmoOrientation::Global, GizmoOrientation::Local] {
                        ui.selectable_value(
                            &mut gizmo_options.gizmo_orientation,
                            o,
                            format!("{:?}", o),
                        );
                    }
                });

            ui.add_space(6.0);

            ui.label(egui::RichText::new("Modes").strong());

            egui::Grid::new("gizmo_modes_grid")
                .num_columns(5)
                .spacing([4.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    for label in ["X", "Y", "Z", "View/Uniform"] {
                        ui.label(egui::RichText::new(label).small().strong());
                    }
                    ui.end_row();

                    ui.label("Rotate");
                    for mode in [
                        GizmoMode::RotateX,
                        GizmoMode::RotateY,
                        GizmoMode::RotateZ,
                        GizmoMode::RotateView,
                    ] {
                        let mut on = gizmo_options.gizmo_modes.contains(mode);
                        if ui.checkbox(&mut on, "").changed() {
                            if on {
                                gizmo_options.gizmo_modes.insert(mode);
                            } else {
                                gizmo_options.gizmo_modes.remove(mode);
                            }
                        }
                    }
                    ui.end_row();

                    ui.label("Translate");
                    for mode in [
                        GizmoMode::TranslateX,
                        GizmoMode::TranslateY,
                        GizmoMode::TranslateZ,
                        GizmoMode::TranslateView,
                    ] {
                        let mut on = gizmo_options.gizmo_modes.contains(mode);
                        if ui.checkbox(&mut on, "").changed() {
                            if on {
                                gizmo_options.gizmo_modes.insert(mode);
                            } else {
                                gizmo_options.gizmo_modes.remove(mode);
                            }
                        }
                    }
                    ui.end_row();

                    ui.label("Scale");
                    for mode in [
                        GizmoMode::ScaleX,
                        GizmoMode::ScaleY,
                        GizmoMode::ScaleZ,
                        GizmoMode::ScaleUniform,
                    ] {
                        let mut on = gizmo_options.gizmo_modes.contains(mode);
                        if ui.checkbox(&mut on, "").changed() {
                            if on {
                                gizmo_options.gizmo_modes.insert(mode);
                            } else {
                                gizmo_options.gizmo_modes.remove(mode);
                            }
                        }
                    }
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.separator();

            ui.label(egui::RichText::new("Manual").strong());
            egui::Grid::new("scene_cfg_manual")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Scale (uniform):");
                    let mut s = scene_transform.transform.scale.x;
                    if ui
                        .add(
                            egui::DragValue::new(&mut s)
                                .speed(0.01)
                                .range(0.001_f32..=f32::MAX)
                                .fixed_decimals(3),
                        )
                        .changed()
                    {
                        scene_transform.transform.scale = Vec3::splat(s.max(0.001));
                    }
                    ui.end_row();
                });

            ui.add_space(4.0);
            if ui.button("Reset transform").clicked() {
                *scene_transform = SceneTransformConfig::default();
            }

            ui.add_space(6.0);
            ui.separator();

            ui.label(egui::RichText::new("3D Text").strong());
            let show_text3d_entity = show_text3d_query.single().ok();
            let show_text3d = show_text3d_entity.is_some();
            if ui.selectable_label(show_text3d, "Show 3D Text").clicked() {
                if let Some(entity) = show_text3d_entity {
                    commands.entity(entity).despawn();
                } else {
                    commands.spawn(ShowText3d);
                }
            }
            ui.horizontal(|ui| {
                ui.label("Renderer:");
                if ui
                    .selectable_label(*text3d_renderer == Text3dRenderer::FontMesh, "FontMesh")
                    .clicked()
                {
                    *text3d_renderer = Text3dRenderer::FontMesh;
                }
                if ui
                    .selectable_label(*text3d_renderer == Text3dRenderer::SlugText, "SlugText")
                    .clicked()
                {
                    *text3d_renderer = Text3dRenderer::SlugText;
                }
            });
            // Bind the textbox to the ecs staging resource so the field is
            // fully responsive and just writes to a dum af string from its
            // pov. Spawning in tick was a baaaad idea and typing fast made
            // things go boom needlessly.
            let mut pending_buf = text3d_pending.0.clone();
            ui.horizontal(|ui| {
                ui.label("Default Text:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut pending_buf)
                        .hint_text("mitchty.github.io")
                        .desired_width(160.0),
                );
                if resp.changed() {
                    text3d_pending.0 = pending_buf;
                }
            });
        });

    Ok(())
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
        Query<Entity, With<HueAnimation>>,
        Query<Entity, With<ShowRecognizer>>,
        Query<Entity, With<ShowWorldClock>>,
        Query<&mut Visibility, With<flan::PlotUiNode>>,
        Query<Entity, With<ShowSceneConfig>>,
    )>,
    #[cfg(not(target_arch = "wasm32"))] data_viewer_query: Query<Entity, With<ShowDataViewer>>,
    mut active_reverie: ResMut<ActiveReverie>,
    reverie_query: Query<(Entity, &ReverieKey, &ReverieDisplayName)>,
    mut active_shader: ResMut<ActiveShader>,
    available_shaders: Res<AvailableShaders>,
    mut commands: Commands,
    mut ui_config: ResMut<UiConfig>,
    // TODO: I really need a better approach to this glorified global config
    // thing for now hacks it is.
    mut proj_params: ParamSet<(Res<CameraMode>, ResMut<CameraProjectionToggleRequested>)>,
    mut reset_camera_events: MessageWriter<ResetCamera>,
    #[cfg(debug_assertions)] mut plugin_registry: ResMut<PluginRegistry>,
) -> Result {
    if show_egui_query.is_empty() {
        return Ok(());
    }

    trace!("settings_ui running - ShowEgui exists");

    egui::TopBottomPanel::top("menu_bar").show(contexts.ctx_mut()?, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            // File menu - native only now (just Quit remains; scene loading moved to Scene sidebar).
            #[cfg(not(target_arch = "wasm32"))]
            ui.menu_button("File", |ui| {
                if ui.button("Quit").clicked() {
                    std::process::exit(0);
                }
            });

            // Scene menu simply (de)toggles the Scene Config side panel.
            let scene_cfg_open = marker_queries.p5().single().is_ok();
            if ui.selectable_label(scene_cfg_open, "Scene").clicked() {
                if let Ok(entity) = marker_queries.p5().single() {
                    commands.entity(entity).despawn();
                } else {
                    commands.spawn(ShowSceneConfig);
                }
            }

            ui.menu_button("Gooey", |ui| {
                ui.label(egui::RichText::new("Projection").strong());
                let proj_label = match *proj_params.p0() {
                    CameraMode::Perspective => "Perspective [M]",
                    CameraMode::Orthographic => "Orthographic [M]",
                };
                if ui.button(proj_label).clicked() {
                    proj_params.p1().0 = true;
                }

                if ui.button("Reset Camera").clicked() {
                    reset_camera_events.write(ResetCamera);
                    ui.close();
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

                let hue_entity = marker_queries.p1().single().ok();
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

            {
                let entries: Vec<_> = reverie_query.iter().collect();
                crate::plugins::reveries::reveries_egui_menu(ui, &entries, &mut active_reverie);
            }

            ui.menu_button("Apps", |ui| {
                let wc_entity = marker_queries.p3().single().ok();
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

                // Debug menu only visible in debug builds for now, obviously.
                // But this is to start to make things be dynamic to toggle
                // things on/off at runtime or to swap between egui
                // implementations and bevy feathers at some point.
                //
                // Not something anyone but me needs to give a shit about, its
                // more so I don't keep half baked stuff in long running
                // branches forever. I hate merge conflicts and its almost
                // summer. Better to chip off bit by bit whilst I keep working
                // things working.
                //
                // All plugins are registered with PluginRegistry. Checkboxes
                // write directly into the registry and
                // `sync_registry_to_plugins` in PreUpdate propagates the flags
                // to each `PluginEnabled<T>` resource before the next Update
                // run. This lets me do feature testing at runtime and to swap
                // things to test.
                #[cfg(debug_assertions)]
                ui.menu_button("Debug", |ui| {
                    ui.label(egui::RichText::new("Plugin Toggles").strong());
                    ui.separator();
                    if plugin_registry.entries.is_empty() {
                        ui.label(
                            egui::RichText::new("No plugins registered.")
                                .italics()
                                .weak(),
                        );
                    } else {
                        for entry in plugin_registry.entries.iter_mut() {
                            ui.checkbox(&mut entry.enabled, entry.name);
                        }
                    }
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
                        .p4()
                        .single()
                        .map(|v| *v != Visibility::Hidden)
                        .unwrap_or(false);
                    if ui
                        .selectable_label(line_graph_visible, "Line Graph")
                        .clicked()
                    {
                        if let Ok(mut vis) = marker_queries.p4().single_mut() {
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
/// on the `MainCamera` entity in-place, preserving the apparent zoom level so
/// the scene looks as similar as possible immediately after the switch between
/// the projection types.
///
/// **Perspective to Orthographic**
/// The orthographic `scale` is derived from the current orbit radius and the
/// perspectives vertical FOV roughly:
///   `ortho_scale = 2 * radius * tan(fov / 2)`
/// This makes the orthographic viewport height match what the perspective
/// camera was framing at the orbit center.
///
/// **Orthographic to Perspective**
/// The inverse to perspective due to orthographic maths. the orbit radius is
/// recomputed from the current ortho scale so the perspective camera lands at
/// the matching distance, then the camera transform is repositioned on the
/// sphere.
pub fn apply_camera_projection_toggle(
    mut events: MessageReader<ToggleCameraProjection>,
    mut camera_mode: ResMut<CameraMode>,
    mut camera_query: Query<
        (
            &mut Projection,
            &mut crate::CameraOrbit,
            &crate::FreeLookCamera,
            &mut Transform,
        ),
        With<MainCamera>,
    >,
) {
    for _ in events.read() {
        let Ok((mut projection, mut orbit, free_look, mut transform)) = camera_query.single_mut()
        else {
            continue;
        };

        let new_mode = match *camera_mode {
            CameraMode::Perspective => {
                // Derive ortho scale from the current perspectives FOV and
                // radius such that the scene appears at roughly the same zoom
                // level after the switch. (haven't full validated the maths its
                // late)
                let fov = match *projection {
                    Projection::Perspective(ref p) => p.fov,
                    _ => PerspectiveProjection::default().fov,
                };
                let ortho_scale = 2.0 * orbit.radius * (fov / 2.0).tan();

                *projection = Projection::Orthographic(OrthographicProjection {
                    scale: ortho_scale,
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
                // Derive the perspective orbit radius from the current ortho
                // scale so the camera ends up at the matching distance from the
                // scene origin.
                let fov = PerspectiveProjection::default().fov;
                let ortho_scale = match *projection {
                    Projection::Orthographic(ref o) => o.scale,
                    _ => 5.0,
                };
                orbit.radius = ortho_scale / (2.0 * (fov / 2.0).tan());

                let x = orbit.center.x + orbit.radius * free_look.yaw.cos() * free_look.pitch.cos();
                let y = orbit.center.y + orbit.radius * free_look.pitch.sin();
                let z = orbit.center.z + orbit.radius * free_look.yaw.sin() * free_look.pitch.cos();
                transform.translation = Vec3::new(x, y, z);
                *transform = transform.looking_at(orbit.center, Vec3::Y);

                *projection = Projection::Perspective(PerspectiveProjection::default());
                CameraMode::Perspective
            }
        };
        bevy::log::debug!("projection now {:?}", new_mode);
        *camera_mode = new_mode;
    }
}

/// Restore the camera to its default position in the case where you lose track
/// where anything is or what you did. Which neeeever happens.
pub fn reset_camera(
    mut events: MessageReader<ResetCamera>,
    mut camera_mode: ResMut<CameraMode>,
    mut camera_query: Query<
        (
            &mut Transform,
            &mut crate::CameraOrbit,
            &mut crate::FreeLookCamera,
            &mut Projection,
        ),
        With<MainCamera>,
    >,
) {
    for _ in events.read() {
        let Ok((mut transform, mut orbit, mut free_look, mut projection)) =
            camera_query.single_mut()
        else {
            continue;
        };

        let defaults = crate::fullscreen_effect::CameraConfig::default();

        *transform = defaults.transform;
        *orbit = defaults.orbit;
        *free_look = defaults.free_look;
        *projection = Projection::Perspective(PerspectiveProjection::default());
        *camera_mode = CameraMode::Perspective;

        bevy::log::debug!("user reset camera to default");
    }
}
