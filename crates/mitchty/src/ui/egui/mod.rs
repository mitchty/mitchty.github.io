mod about;
mod apps;
#[cfg(all(dev_build, not(target_arch = "wasm32")))]
mod debug;
mod experiments;
#[cfg(not(target_arch = "wasm32"))]
mod file;
mod gooey;
mod reveries;
mod scene;
mod theme_toggle;

use crate::CameraMode;
use crate::ai::infer::InferenceEngine;
use crate::plugins::camera::MainCamera;
use crate::plugins::fonts::{PendingFontRegistration, RegisteredFonts};
use crate::plugins::fps::FpsDisplay;
use crate::plugins::hue::HueAnimation;
use crate::plugins::reveries::{ActiveReverie, ReverieDisplayName, ReverieKey};
use crate::plugins::scene::{
    ColorState, SceneConfig, SceneTransformConfig, SceneUrlState, ShowSceneModel,
};
use crate::plugins::text3d::{
    FlanFontId, SlugText3dFontValidation, SlugText3dState, Text3dRenderer,
};
use crate::ui::config::UiConfig;
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::data_viewer::{
    DataViewerState, ImageProcessingCache, ShowDataViewer, data_viewer_window,
};
use crate::ui::losant::{
    LosantAuthTask, LosantDiscoveryTask, LosantSseChannel, LosantSseTask, LosantState,
};
use crate::ui::losant::{poll_losant_auth_task, poll_losant_discovery_task, poll_losant_sse};
#[cfg(dev_build)]
use crate::ui::recognizer::RasterSize;
use crate::ui::recognizer::{BASE_BRUSH_R, InferenceResult, RecognizerState};
use crate::ui::state::{UiBackend, UiPanel, UiState, egui_backend_active};
use crate::ui::world_clock::{ShowWorldClock, WorldClockState, world_clock_window};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
#[cfg(not(target_arch = "wasm32"))]
use egui_file_dialog::FileDialog;
use flan::post_process::{ActiveShader, AvailableShaders, EffectsEnabled};
use mitchty::ActiveApp;
use transform_gizmo_bevy::prelude::{GizmoMode, GizmoOptions, GizmoOrientation};

use crate::plugins::theme::{EguiThemeSet, ThemePlugin, resolve_initial_theme};
use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};

/// Marker component attached alongside `UiPanel` to identify entities that
/// contribute an egui menu bar entry.
///
/// The `UiPanel` component carries the backend-agnostic metadata (id, anchor,
/// order). This component is the egui-specific signal that the orchestrator
/// (`settings_ui`) should include this panel in the egui menu bar.
///
/// TODO: Extend with `render: fn(&mut egui::Ui, &mut World)` once `settings_ui`
/// is converted to an exclusive system, enabling full fn-pointer dispatch and
/// removing the direct `about::render_about_menu` / `reveries::render_reveries_menu`
/// calls from the orchestrator.
#[derive(Component)]
pub struct EguiMenuBarItem;

/// Cached wgpu backend and adapter name. Populated once by `cache_adapter_info`
/// as soon as `RenderAdapterInfo` becomes available after render-world init.
/// Stored as a `Resource` so `settings_ui` can read it directly without adding
/// another `SystemParam` cause I'm already abusing params and bouncing off the
/// limit as it is. I need to learn a better approach for all this rigamarole.
#[derive(Resource, Default)]
pub struct CachedAdapterInfo(pub Option<String>);

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

/// Marker component that is spawned by `load_egui_noto_font` at startup and
/// despawned by `configure_egui_style` once font/theme setup completes.
#[derive(Component)]
pub struct EguiStyleConfiguring;

/// Non-Send wrapper around `egui_file_dialog::FileDialog`, Resource needs to
/// match Send to work.
#[cfg(not(target_arch = "wasm32"))]
pub struct SceneFileDialog(pub FileDialog);

/// Wrapper marker component for font file picking dialog.
#[cfg(not(target_arch = "wasm32"))]
pub struct FontFileDialog(pub FileDialog);

/// Track font assets added at runtime. Each entry is `(display_name, handle)`
/// where `display_name` is the font file name stem aka `"MyFont.ttf"` and the
/// handle points to a `bevy::text::Font` asset in the Bevy asset server.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Default)]
pub struct DynamicFontHandles(pub Vec<(String, Handle<Font>)>);

/// Bundle the native-only (for now) file-dialog and dynamic-font params.
#[cfg(not(target_arch = "wasm32"))]
#[derive(bevy::ecs::system::SystemParam)]
pub struct FontPanelParams<'w> {
    pub font_file_dialog: NonSendMut<'w, FontFileDialog>,
    pub dynamic_fonts: ResMut<'w, DynamicFontHandles>,
    pub asset_server: Res<'w, AssetServer>,
    pub embedded_fonts: Option<Res<'w, EmbeddedFontHandles>>,
}

/// Bundle struct for reverie list and active state into one SystemParam so
/// settings_ui stays within Bevy's 16-parameter limit as it's already at
/// capacity on native builds. Need to do some refactoring on this stuff but
/// probably just will keep tackling this stuff with hacks until I can ditch
/// egui entirely.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ReverieParams<'w, 's> {
    pub active: ResMut<'w, ActiveReverie>,
    pub entries: Query<'w, 's, (Entity, &'static ReverieKey, &'static ReverieDisplayName)>,
}

/// Bundle 3d text font-picker state.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Text3dFontParams<'w> {
    pub registered: Res<'w, RegisteredFonts>,
    pub validation: Res<'w, SlugText3dFontValidation>,
    pub active_font: Option<Res<'w, FlanFontId>>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct AppSwitchParams<'w> {
    pub active_app: Res<'w, ActiveApp>,
    pub pending: ResMut<'w, PendingAppSwitch>,
    #[cfg(all(dev_build, not(target_arch = "wasm32")))]
    pub plugin_registry: ResMut<'w, PluginRegistry>,
}

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

/// Staging resource for pending ActiveApp switch.
#[derive(Resource, Default)]
pub struct PendingAppSwitch(pub Option<ActiveApp>);

/// Marker component to track whether the Recognizer window is open
#[derive(Component)]
pub struct ShowRecognizer;

/// Marker component to track whether the Scene Config window is open
#[derive(Component)]
pub struct ShowSceneConfig;

/// Marker component to track whether the Losant window is open
#[derive(Component)]
pub struct ShowLosant;

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
        // compiled-in default during development.
        // Note: explicitly exclude wasm32 InferenceEngine::load is native-only for now.
        #[cfg(all(dev_build, not(target_arch = "wasm32")))]
        let engine = engine.or_else(|| {
            ["recognizer", "../../../recognizer", "../recognizer"]
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
                SettingsUiSystems
                    .run_if(run_if_enabled::<SettingsUiPlugin>())
                    .run_if(egui_backend_active()),
            )
            .configure_sets(
                EguiPrimaryContextPass,
                SettingsUiSystems
                    .run_if(run_if_enabled::<SettingsUiPlugin>())
                    .run_if(egui_backend_active()),
            )
            .init_resource::<UiState>()
            .add_plugins(ThemePlugin)
            .add_plugins(about::AboutMenuPlugin)
            .add_plugins(apps::AppsMenuPlugin)
            .add_plugins(experiments::ExperimentsMenuPlugin)
            .add_plugins(gooey::GooeyMenuPlugin)
            .add_plugins(reveries::ReveriesMenuPlugin)
            .add_plugins(scene::SceneMenuPlugin)
            .add_plugins(theme_toggle::ThemeToggleMenuPlugin)
            .init_resource::<CachedAdapterInfo>()
            .init_resource::<EguiWantsInput>()
            .init_resource::<LosantState>()
            .init_resource::<LosantAuthTask>()
            .init_resource::<LosantDiscoveryTask>()
            .init_resource::<LosantSseTask>()
            .init_resource::<LosantSseChannel>()
            .init_resource::<RecognizerState>()
            .init_resource::<InferenceResult>()
            .add_message::<ToggleCameraProjection>()
            .add_message::<ResetCamera>()
            .add_message::<crate::ui::losant::DeviceStateEvent>()
            .init_resource::<CameraProjectionToggleRequested>()
            .init_resource::<PendingAppSwitch>()
            .insert_non_send(engine)
            .add_systems(Startup, (setup_egui, load_egui_noto_font))
            .add_systems(
                Update,
                (
                    send_camera_projection_toggle,
                    apply_camera_projection_toggle,
                    reset_camera,
                    apply_pending_app_switch,
                )
                    .chain()
                    .in_set(SettingsUiSystems),
            );

        let app = app.add_systems(
            Update,
            (
                poll_losant_auth_task,
                poll_losant_discovery_task,
                poll_losant_sse,
            )
                .in_set(SettingsUiSystems),
        );

        #[cfg(not(target_arch = "wasm32"))]
        let app = app
            .init_resource::<DataViewerState>()
            .init_resource::<ImageProcessingCache>()
            .init_resource::<DynamicFontHandles>()
            .insert_non_send(SceneFileDialog(
                FileDialog::new()
                    .title("Load Scene GLTF")
                    .add_file_filter_extensions("GLTF / GLB", vec!["gltf", "glb"]),
            ))
            .insert_non_send(FontFileDialog(
                FileDialog::new()
                    .title("Add Font")
                    .add_file_filter_extensions("Font files", vec!["ttf", "otf"])
                    .default_file_filter("Font files"),
            ));

        app.add_systems(
            Update,
            cache_adapter_info.run_if(|c: Res<CachedAdapterInfo>| c.0.is_none()),
        );

        app.add_systems(
            EguiPrimaryContextPass,
            configure_egui_style
                .run_if(|q: Query<(), With<EguiStyleConfiguring>>| !q.is_empty())
                .in_set(EguiThemeSet)
                .in_set(SettingsUiSystems),
        );
        app.add_systems(
            EguiPrimaryContextPass,
            settings_ui.in_set(SettingsUiSystems),
        );
        app.add_systems(
            EguiPrimaryContextPass,
            (
                recognizer_window,
                #[cfg(not(target_arch = "wasm32"))]
                data_viewer_window,
                world_clock_window,
                scene_config_window,
                losant_window,
                update_egui_input_state,
            )
                .chain()
                .in_set(SettingsUiSystems),
        );

        #[cfg(not(target_arch = "wasm32"))]
        app.add_plugins(file::FileMenuPlugin);

        #[cfg(all(dev_build, not(target_arch = "wasm32")))]
        app.add_plugins(debug::DebugMenuPlugin);

        if let Some(mut registry) = app.world_mut().get_resource_mut::<PluginRegistry>() {
            registry.register::<SettingsUiPlugin>("Settings UI", true);
        }
    }
}

/// Persistent strong handle for the NotoSansJP font used by egui's
/// `configure_egui_style` to set the CJK fallback once it loads.
#[derive(Resource)]
pub struct EguiNotoFontHandle(pub Handle<Font>);

/// Persistent strong handles for the two always-present embedded fonts.
///
/// These must live for the lifetime of the app so Bevy keeps the assets alive.
#[derive(Resource)]
pub struct EmbeddedFontHandles {
    pub noto: Handle<Font>,
    pub fira: Handle<Font>,
}

/// Startup system to load embedded fonts into the asset server (for now) and
/// let egui use the raw font data as well.
///
/// Also spawns the `EguiStyleConfiguring` marker so that `configure_egui_style`
/// knows asset setup is still running.
pub fn load_egui_noto_font(mut commands: Commands, asset_server: Res<AssetServer>) {
    use crate::assets::asset_path;
    // TODO: build.rs lazy bum
    let noto = asset_server.load(asset_path("fonts/NotoSansJP-Regular.ttf"));
    let fira = asset_server.load(asset_path("fonts/FiraMono-Medium.ttf"));
    commands.insert_resource(EguiNotoFontHandle(noto.clone()));
    commands.insert_resource(EmbeddedFontHandles {
        noto: noto.clone(),
        fira: fira.clone(),
    });
    commands.spawn(PendingFontRegistration {
        name: "NotoSansJP-Regular.ttf".to_string(),
        handle: noto,
    });
    commands.spawn(PendingFontRegistration {
        name: "FiraMono-Medium.ttf".to_string(),
        handle: fira,
    });
    commands.spawn(EguiStyleConfiguring);
}

/// Default model config JSON for the default model for now, future will be to
/// be able to pick different models.
static DEFAULT_MODEL_CONFIG: &[u8] = include_bytes!("../../assets/models/default/config.json");

/// Default model weights for ^^^
static DEFAULT_MODEL_WEIGHTS: &[u8] = include_bytes!("../../assets/models/default/model.mpk");

/// Bump up egui text, register NotoSansJP as a CJK fallback, and apply the
/// resolved startup theme. Note only runs until the font asset finishes
/// loading.
fn configure_egui_style(
    mut contexts: EguiContexts,
    ui_config: Res<UiConfig>,
    noto_handle: Option<Res<EguiNotoFontHandle>>,
    font_assets: Res<Assets<Font>>,
    configuring_query: Query<Entity, With<EguiStyleConfiguring>>,
    mut commands: Commands,
) -> Result {
    // Wait until the font bytes are available before finishing setup.
    //
    // Early return so we keep running until `EguiStyleConfiguring` is
    // despawned at the end.
    let Some(handle) = noto_handle else {
        return Ok(());
    };
    let Some(font) = font_assets.get(&handle.0) else {
        return Ok(());
    };

    let ctx = contexts.ctx_mut()?;

    // Apply the startup theme before anything else so there is no flash of the
    // wrong theme while fonts are being set up.
    ctx.set_visuals(resolve_initial_theme(ui_config.theme));

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "NotoSansJP".to_owned(),
        egui::FontData::from_owned(font.data.data().to_vec()).into(),
    );

    // Only use noto sans jp for anything that fails to render in a latin subset
    for family_data in fonts.families.values_mut() {
        family_data.push("NotoSansJP".to_owned());
    }
    ctx.set_fonts(fonts);

    ctx.global_style_mut(|style| {
        for font_id in style.text_styles.values_mut() {
            font_id.size += 4.0;
        }
    });

    // Ensure this system doesn't run again, it doesn't need to run again now.
    if let Ok(entity) = configuring_query.single() {
        commands.entity(entity).despawn();
    }

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
    mut effects_enabled: Option<ResMut<EffectsEnabled>>,
    mut active_reverie: ResMut<ActiveReverie>,
    ui_config: Res<UiConfig>,
    reveries: Query<(Entity, &ReverieKey, &ReverieDisplayName)>,
    mut ui_state: ResMut<UiState>,
    panel_query: Query<&UiPanel>,
) {
    // Start with effects enabled by default nop when PostProcessPlugin is disabled.
    if let Some(ref mut enabled) = effects_enabled {
        enabled.0 = true;
    }

    // Initialize the initial UiState, `settings_ui` reads this every frame to
    // decide what to draw and how. This is a bit silly for now but it'll make
    // more sense in post/later.
    ui_state.backend = UiBackend::Egui;
    ui_state.menu_bar_visible = ui_config.show_menu_bar;
    ui_state.enabled = true;

    // Log every registered UiPanel so it's easy to verify the full set at
    // startup. Also exercises id/anchor/order so the compiler sees them as read
    // and doesn't optimize them out like a demon.
    // TODO: Gate this to debug only builds?
    for panel in panel_query.iter() {
        debug!(
            "ui panel registered: id={} anchor={:?} order={}",
            panel.id, panel.anchor, panel.order
        );
    }

    if ui_config.show_fps {
        commands.spawn(FpsDisplay);
    }

    // ShowEgui is mostly kept for update_egui_input_state which still uses the
    // marker component for now settings_ui now gates on UiState further
    // refactors can fix this.
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

    if ui_config.show_losant {
        commands.spawn(ShowLosant);
    }

    // Resolve the raw initial_reverie string -> Entity mappings
    if let Some(ref name) = ui_config.initial_reverie {
        crate::plugins::reveries::apply_initial_reverie(name, &reveries, &mut active_reverie);
    }

    // Build WorldClockState from UiConfig overrides. insert_resource replaces
    // the defaults for startup if provided from switches.
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
    show_scene_model_query: Query<Entity, With<ShowSceneModel>>,
    mut scene_transform: ResMut<SceneTransformConfig>,
    mut gizmo_options: ResMut<GizmoOptions>,
    mut text3d_state: ResMut<SlugText3dState>,
    mut scene_config: ResMut<SceneConfig>,
    mut scene_url_state: ResMut<SceneUrlState>,
    #[cfg(not(target_arch = "wasm32"))] mut scene_file_dialog: NonSendMut<SceneFileDialog>,
    #[cfg(not(target_arch = "wasm32"))] mut font_params: FontPanelParams<'_>,
    #[cfg(target_arch = "wasm32")] asset_server: Res<AssetServer>,
    #[cfg(target_arch = "wasm32")] embedded_fonts: Option<Res<EmbeddedFontHandles>>,
    font3d_params: Text3dFontParams<'_>,
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

        // Drive font file picker dialog, load picked font into the asset server.
        font_params.font_file_dialog.0.update(ctx);
        if let Some(path) = font_params.font_file_dialog.0.take_picked() {
            let display_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let handle: Handle<Font> = font_params
                .asset_server
                .load(path.to_string_lossy().into_owned());
            font_params
                .dynamic_fonts
                .0
                .push((display_name.clone(), handle.clone()));
            commands.spawn(PendingFontRegistration {
                name: display_name,
                handle,
            });
        }
    }

    // URL popup + 2-phase commit logic for the scene load setup.
    draw_scene_url_popup(contexts.ctx_mut()?, &mut scene_url_state);
    if let Some(url) = scene_url_state.confirmed_url.take() {
        scene_config.custom_scene = Some(url);
    }

    #[allow(deprecated)]
    egui::Panel::left("scene_config_panel")
        .resizable(true)
        .default_size(240.0)
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
            // Visibility toggle write state.visible directly, debounce fires SlugText3dApply later.
            if ui
                .selectable_label(text3d_state.visible, "Show 3D Text")
                .clicked()
            {
                text3d_state.visible = !text3d_state.visible;
            }

            // Renderer selector.
            ui.horizontal(|ui| {
                ui.label("Renderer:");
                if ui
                    .selectable_label(
                        text3d_state.renderer == Text3dRenderer::SlugText3d,
                        "SlugText3d",
                    )
                    .clicked()
                    && text3d_state.renderer != Text3dRenderer::SlugText3d
                {
                    text3d_state.renderer = Text3dRenderer::SlugText3d;
                }
                if ui
                    .selectable_label(
                        text3d_state.renderer == Text3dRenderer::SlugText,
                        "SlugText",
                    )
                    .clicked()
                    && text3d_state.renderer != Text3dRenderer::SlugText
                {
                    text3d_state.renderer = Text3dRenderer::SlugText;
                }
            });

            // Font picker writes state.font_id and apply_slug_text3d validates next debounce.
            {
                let registered = &font3d_params.registered.0;
                let active_id = font3d_params.active_font.as_deref().map(|f| f.0);
                let selected_label = active_id
                    .and_then(|id| registered.iter().find(|e| e.font_id == id))
                    .map(|e| e.name.as_str())
                    .unwrap_or("(none)");
                ui.horizontal(|ui| {
                    ui.label("Font:");
                    egui::ComboBox::from_id_salt("text3d_font_picker")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for entry in registered {
                                let is_selected = active_id == Some(entry.font_id);
                                if ui.selectable_label(is_selected, &entry.name).clicked()
                                    && !is_selected
                                {
                                    text3d_state.font_id = Some(entry.font_id);
                                }
                            }
                        });
                });
                let missing = &font3d_params.validation.missing_glyphs;
                if !missing.is_empty() {
                    let chars: String = missing
                        .iter()
                        .map(|c| format!("{c:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(
                        egui::RichText::new(format!("❌ Missing glyphs: {chars}"))
                            .small()
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
            }

            // Depth drag widget write state.depth directly.
            if text3d_state.renderer == Text3dRenderer::SlugText3d {
                ui.horizontal(|ui| {
                    ui.label("Depth:");
                    let mut depth_val = text3d_state.bypass_change_detection().depth;
                    if ui
                        .add(
                            egui::DragValue::new(&mut depth_val)
                                .speed(0.001)
                                .range(0.001_f32..=1.0)
                                .fixed_decimals(3),
                        )
                        .changed()
                    {
                        text3d_state.depth = depth_val;
                    }
                });
            }

            // Note the debounce system in use here may not be needed any
            // longer as its a leftover/holdover from fontmesh which took
            // forever to generate meshes. The current system is both faster at
            // making Mesh and caches glyphs to make this mostly unneeded. Maybe
            // I make the debounce just 100ms.

            // Default-text textbox writes state.default_text on every keystroke.
            // Any change is immediately visible to the debounce system via is_changed().
            let mut pending_buf = text3d_state.bypass_change_detection().default_text.clone();
            ui.horizontal(|ui| {
                ui.label("Default Text:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut pending_buf)
                        .hint_text("mitchty.github.io")
                        .desired_width(160.0),
                );
                if resp.changed() {
                    text3d_state.default_text = pending_buf.clone();
                    // Also push to live text immediately so there's less gap
                    // between typing and seeing the change reverie sync will
                    // override this each second when it is active need to
                    // rethink that approach and get the node graph system in
                    // place to make a truly dynamic display runtime setup.
                    text3d_state.text = pending_buf;
                }
            });

            // Color picker write state.color directly.
            ui.horizontal(|ui| {
                ui.label("Text Color:");
                let [r, g, b, a] = text3d_state.bypass_change_detection().color;
                let mut egui_color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
                if ui.color_edit_button_srgba(&mut egui_color).changed() {
                    let [r2, g2, b2, a2] = egui_color.to_array();
                    text3d_state.color = [r2, g2, b2, a2];
                }
                const DEFAULT_COLOR: [u8; 4] = [0, 0, 0, 255];
                if ui
                    .add_enabled(
                        text3d_state.bypass_change_detection().color != DEFAULT_COLOR,
                        egui::Button::new("↩ Reset"),
                    )
                    .on_disabled_hover_text("Already default color")
                    .clicked()
                {
                    text3d_state.color = DEFAULT_COLOR;
                }
            });

            ui.add_space(6.0);
            ui.separator();

            ui.label(egui::RichText::new("Fonts").strong());

            // TODO: Get the font picker to accept a uri so wasm can add font files too.
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("📂 Add Font").clicked() {
                font_params.font_file_dialog.0.pick_file();
            }

            // Obtain asset server and embedded font handles regardless of
            // platform. On native they live inside FontPanelParams, on wasm
            // they are direct system params.
            #[cfg(not(target_arch = "wasm32"))]
            let srv = &*font_params.asset_server;
            #[cfg(target_arch = "wasm32")]
            let srv = &*asset_server;

            #[cfg(not(target_arch = "wasm32"))]
            let embedded = font_params.embedded_fonts.as_deref();
            #[cfg(target_arch = "wasm32")]
            let embedded = embedded_fonts.as_deref();

            // Helper: map a bevy LoadState to a short status icon + tooltip.
            let load_state_label = |handle: &Handle<Font>| -> (&'static str, &'static str) {
                use bevy::asset::LoadState;
                match srv.get_load_state(handle) {
                    Some(LoadState::Loaded) => ("✅", "Loaded"),
                    Some(LoadState::Loading) => ("⏳", "Loading..."),
                    Some(LoadState::Failed(_)) => ("❌", "Failed to load"),
                    Some(LoadState::NotLoaded) | None => ("❓", "Not loaded"),
                }
            };

            // Build the list of (display_name, handle_ref) for all known fonts.
            // Embedded fonts use the persistent handles from EmbeddedFontHandles
            // so Bevy always has a strong-handle owner and the asset stays loaded.
            // Calling asset_server.load() every frame and dropping the handle
            // caused FiraMono to stay stuck at ⏳ because no owner kept it alive.
            //
            // TODO: This is all very jank. "I'll fix it in post". Promise...
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("fonts_scroll")
                .max_height(120.0)
                .show(ui, |ui| {
                    if let Some(ef) = embedded {
                        for (name, handle) in [
                            ("NotoSansJP-Regular.ttf", &ef.noto),
                            ("FiraMono-Medium.ttf", &ef.fira),
                        ] {
                            let (icon, tip) = load_state_label(handle);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(icon));
                                ui.label(name)
                                    .on_hover_text(format!("Embedded font - {tip}"));
                            });
                        }
                    }

                    // Dynamically added at runtime fonts come after the embeds.
                    // TODO: How do I handle duplicates? I punted on this cause lazy.
                    #[cfg(not(target_arch = "wasm32"))]
                    for (name, handle) in &font_params.dynamic_fonts.0 {
                        let (icon, tip) = load_state_label(handle);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(icon));
                            ui.label(name.as_str())
                                .on_hover_text(format!("Runtime font - {tip}"));
                        });
                    }
                });
        });

    Ok(())
}

/// Display the settings UI using egui as a top menu bar.
///
/// All per-menu logic lives in the sub-modules (file, scene, gooey, reveries,
/// apps, theme_toggle, about, debug, experiments). This function's job is to:
///   1. Pre-extract ECS state before the egui closure opens.
///   2. Open the menu bar and call each render function in order.
///   3. Write any mutations back to resources after the closure.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn settings_ui(
    mut contexts: EguiContexts,
    mut color_state: ResMut<ColorState>,
    mut effects_enabled: Option<ResMut<EffectsEnabled>>,
    ui_state: Res<UiState>,
    cached_adapter_info: Res<CachedAdapterInfo>,
    mut marker_queries: ParamSet<(
        Query<Entity, With<FpsDisplay>>,                            // p0
        Query<Entity, With<HueAnimation>>,                          // p1
        Query<Entity, With<ShowRecognizer>>,                        // p2
        Query<Entity, With<ShowWorldClock>>,                        // p3
        Query<Entity, With<crate::plugins::fps::StatsOverlayNode>>, // p4 unused slot for now kept for future refactor
        Query<Entity, With<ShowSceneConfig>>,                       // p5
    )>,
    #[cfg(not(target_arch = "wasm32"))] data_viewer_query: Query<Entity, With<ShowDataViewer>>,
    losant_query: Query<Entity, With<ShowLosant>>,
    mut reverie_params: ReverieParams,
    mut active_shader: Option<ResMut<ActiveShader>>,
    available_shaders: Option<Res<AvailableShaders>>,
    mut commands: Commands,
    mut ui_config: ResMut<UiConfig>,
    mut camera_proj: ParamSet<(Res<CameraMode>, ResMut<CameraProjectionToggleRequested>)>,
    mut reset_camera_events: MessageWriter<ResetCamera>,
    mut app_switch: AppSwitchParams<'_>,
) -> Result {
    if !ui_state.enabled || !ui_state.menu_bar_visible {
        return Ok(());
    }

    trace!("settings_ui running");

    // TODO: I hate this pN bs soo bad when I need to change the paramset count
    // and order.
    // p0 FpsDisplay, p1 HueAnimation
    let fps_entity = marker_queries.p0().single().ok();
    let hue_entity = marker_queries.p1().single().ok();
    // p2 ShowRecognizer
    let recognizer_entity = marker_queries.p2().single().ok();
    // p3 ShowWorldClock
    let clock = marker_queries.p3().single().ok();
    // p4 StatsOverlayNode slot kept for index stability not used in gooey can remove but this commits gihugic enough as it is and I don't want to change pN things right now the diff for this craps huge.
    let _overlay_entity = marker_queries.p4().single().ok();
    // p5 ShowSceneConfig
    let scene_cfg_entity = marker_queries.p5().single().ok();

    #[cfg(not(target_arch = "wasm32"))]
    let data_viewer_entity = data_viewer_query.single().ok();
    let losant_entity = losant_query.single().ok();

    let reverie_entries: Vec<_> = reverie_params.entries.iter().collect();

    // Gooey mutable data's render fn writes back via this struct, caller
    // applies to the real resources.
    let current_camera_mode = *camera_proj.p0();
    let mut gooey_data = gooey::GooeyRenderData {
        fps_entity,
        hue_entity,
        camera_mode: current_camera_mode,
        proj_toggle_requested: false,
        effects_enabled: effects_enabled.as_deref().map(|e| e.0),
        active_shader_index: active_shader.as_deref().map(|s| s.index),
        shader_entries: available_shaders.as_ref().map(|a| {
            a.shaders
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.display_name.clone()))
                .collect()
        }),
        color: color_state.color,
        theme: ui_config.theme,
    };

    #[allow(deprecated)]
    egui::Panel::top("menu_bar").show(contexts.ctx_mut()?, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            // lhs of the menubar
            #[cfg(not(target_arch = "wasm32"))]
            file::render_file_menu(ui);

            scene::render_scene_menu(
                ui,
                scene::SceneRenderData { scene_cfg_entity },
                &mut commands,
            );

            gooey::render_gooey_menu(ui, &mut gooey_data, &mut commands, &mut reset_camera_events);

            {
                reveries::render_reveries_menu(ui, &reverie_entries, &mut reverie_params.active);
            }

            {
                let current_app = *app_switch.active_app;
                let mut next_app = current_app;
                apps::render_apps_menu(
                    ui,
                    apps::AppsRenderData {
                        clock,
                        active_app: current_app,
                    },
                    &mut commands,
                    &mut next_app,
                );
                if next_app != current_app {
                    app_switch.pending.0 = Some(next_app);
                }
            }

            // rhs of the menubar
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme_toggle::render_theme_toggle_menu(ui, &mut ui_config);
                about::render_about_menu(ui, cached_adapter_info.0.as_deref());
                #[cfg(all(dev_build, not(target_arch = "wasm32")))]
                debug::render_debug_menu(ui, &mut app_switch.plugin_registry);
                experiments::render_experiments_menu(
                    ui,
                    experiments::ExperimentsRenderData {
                        recognizer_entity,
                        line_graph: None, // TODO: plot things gone need to rip this old crap out fully
                        #[cfg(not(target_arch = "wasm32"))]
                        data_viewer_entity,
                        losant_entity,
                    },
                    &mut commands,
                );
                egui::warn_if_debug_build(ui);
            });
        });
    });

    if gooey_data.proj_toggle_requested {
        camera_proj.p1().0 = true;
    }
    if let (Some(ref mut enabled), Some(new_val)) =
        (effects_enabled.as_deref_mut(), gooey_data.effects_enabled)
    {
        enabled.0 = new_val;
    }
    if let (Some(ref mut shader), Some(shaders), Some(new_idx)) = (
        active_shader.as_deref_mut(),
        available_shaders.as_deref(),
        gooey_data.active_shader_index,
    ) && shader.index != new_idx
    {
        shader.index = new_idx;
        trace!("shader effect changed to {}", shader.display_name(shaders));
    }
    color_state.color = gooey_data.color;
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

                // Size picker only in dev builds, here to let me test
                // models trained at different input resolutions without
                // recompiling. Will rip this out eventually?
                #[cfg(dev_build)]
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
                        egui::Stroke::new(1.5_f32, egui::Color32::RED),
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

/// System to render the Losant egui window when ShowLosant is present.
#[allow(clippy::too_many_arguments)]
fn losant_window(
    mut contexts: EguiContexts,
    losant_query: Query<Entity, With<ShowLosant>>,
    mut state: ResMut<LosantState>,
    mut auth_task: ResMut<LosantAuthTask>,
    mut discovery_task: ResMut<LosantDiscoveryTask>,
    mut sse_task: ResMut<LosantSseTask>,
    mut sse_channel: ResMut<LosantSseChannel>,
    mut commands: Commands,
) -> Result {
    use crate::ui::losant::{
        LosantAuthStatus, LosantDiscoveryStatus, LosantSseStatus, spawn_fetch_applications,
        spawn_fetch_devices, spawn_losant_auth_task, spawn_sse_connect, spawn_sse_disconnect,
    };

    if losant_query.is_empty() {
        return Ok(());
    }

    let mut open = true;
    egui::Window::new("Losant")
        .open(&mut open)
        .default_size([460.0, 520.0])
        .resizable(true)
        .show(contexts.ctx_mut()?, |ui| {
            // I have zero idea how to do this well, so the ui progressively
            // expands as more happens. Login is the only always shown section.
            //
            // TODO: is once I stop being a hack here is to put in a state
            // machine to more directly drive the gui sections versus just
            // booleans all over the way.

            ui.label(egui::RichText::new("GitHub Authentication").strong());
            ui.separator();
            ui.add_space(4.0);

            let authenticated = state.auth_status == LosantAuthStatus::Success;

            if !authenticated {
                ui.label("GitHub Access Token:");
                ui.add_space(2.0);
                ui.add(
                    egui::TextEdit::singleline(&mut state.github_token_input)
                        .hint_text("ghp_abcd...xyz")
                        .password(true)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(6.0);

                let can_auth = matches!(
                    state.auth_status,
                    LosantAuthStatus::Idle | LosantAuthStatus::Error(_)
                ) && !state.github_token_input.trim().is_empty();

                if ui
                    .add_enabled(can_auth, egui::Button::new("Authenticate via GitHub Token"))
                    .clicked()
                {
                    spawn_losant_auth_task(&mut state, &mut auth_task);
                }
                ui.add_space(4.0);
            }

            match &state.auth_status.clone() {
                LosantAuthStatus::Idle => {
                    ui.label(egui::RichText::new("Not authenticated.").weak().italics());
                }
                LosantAuthStatus::InFlight => {
                    ui.label(
                        // TODO: This looks like ass in light theme. I need to
                        // make a `Resource` so I can change this color based
                        // off of active theme. Thats outside of scope for this
                        // tho.
                        egui::RichText::new("⏳ Authenticating hold please")
                            .color(egui::Color32::YELLOW),
                    );
                }
                LosantAuthStatus::Success => {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("✔ Authenticated")
                                .color(egui::Color32::from_rgb(80, 200, 120))
                                .strong(),
                        );
                        if let Some(token) = &state.bearer_token {
                            // Don't show the full token just the prefix and suffix.
                            let preview = if token.len() > 12 {
                                format!("{} truncated {}", &token[..6], &token[token.len() - 4..])
                            } else {
                                "(short)".to_string()
                            };
                            ui.label(egui::RichText::new(format!("({preview})")).small().weak());
                        }
                        if ui.small_button("Clear").clicked() {
                            state.auth_status = LosantAuthStatus::Idle;
                            state.bearer_token = None;
                            state.github_token_input.clear();
                            state.applications.clear();
                            state.selected_application = None;
                            state.devices.clear();
                            state.selected_device = None;
                            state.discovery_status = LosantDiscoveryStatus::Idle;
                            state.sse_status = LosantSseStatus::Disconnected;
                            sse_task.0 = None;
                            sse_channel.0 = None;
                        }
                    });
                }
                LosantAuthStatus::Error(msg) => {
                    let msg = msg.clone();
                    ui.label(
                        egui::RichText::new(format!("✖ Error: {msg}"))
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
            }

            if !authenticated {
                return;
            }

            // Ok iff auth worked, then we can show the app discovery bits and
            // associated devices.
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Device Discovery").strong());
            ui.separator();
            ui.add_space(4.0);

            let bearer = state.bearer_token.clone().unwrap_or_default();
            let fetching = matches!(
                state.discovery_status,
                LosantDiscoveryStatus::FetchingApps | LosantDiscoveryStatus::FetchingDevices
            );

            if ui
                .add_enabled(
                    !fetching,
                    egui::Button::new(if state.applications.is_empty() {
                        "Fetch Applications"
                    } else {
                        "↺ Refresh Applications"
                    }),
                )
                .clicked()
            {
                spawn_fetch_applications(bearer.clone(), &mut state, &mut discovery_task);
            }

            match &state.discovery_status.clone() {
                LosantDiscoveryStatus::FetchingApps => {
                    ui.label(
                        egui::RichText::new("⏳ Fetching applications hold please")
                            .color(egui::Color32::YELLOW),
                    );
                }
                LosantDiscoveryStatus::FetchingDevices => {
                    ui.label(
                        egui::RichText::new("⏳ Fetching devices hold please")
                            .color(egui::Color32::YELLOW),
                    );
                }
                LosantDiscoveryStatus::Error(msg) => {
                    let msg = msg.clone();
                    ui.label(
                        egui::RichText::new(format!("✖ {msg}"))
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
                _ => {}
            }

            // Application picker drop down.
            // Auto-select the only application when there is exactly one choice.
            if state.applications.len() == 1 && state.selected_application.is_none() {
                state.selected_application = Some(0);
                state.devices.clear();
                state.selected_device = None;
            }

            if !state.applications.is_empty() {
                ui.add_space(4.0);
                ui.label("Application:");
                let app_label = state
                    .selected_application
                    .and_then(|i| state.applications.get(i))
                    .map(|a| a.name.as_str())
                    .unwrap_or("select application");

                // Collect names first to avoid holding an immutable borrow
                // on state.applications while also mutating state inside the closure.
                let app_names: Vec<(usize, String)> = state
                    .applications
                    .iter()
                    .enumerate()
                    .map(|(i, a)| (i, a.name.clone()))
                    .collect();
                let mut new_app_sel = None;
                egui::ComboBox::from_id_salt("losant_app_picker")
                    .selected_text(app_label)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for (idx, name) in &app_names {
                            let selected = state.selected_application == Some(*idx);
                            if ui.selectable_label(selected, name).clicked()
                                && state.selected_application != Some(*idx)
                            {
                                new_app_sel = Some(*idx);
                            }
                        }
                    });
                if let Some(idx) = new_app_sel {
                    state.selected_application = Some(idx);
                    state.devices.clear();
                    state.selected_device = None;
                }
            }

            // Fetch Devices button, only usable when an application is selected
            // so things don't get weirder.
            if let Some(app_idx) = state.selected_application
                && let Some(app) = state.applications.get(app_idx)
            {
                let app_id = app.id.clone();
                ui.add_space(4.0);
                if ui
                    .add_enabled(
                        !fetching,
                        egui::Button::new(if state.devices.is_empty() {
                            "Fetch Devices"
                        } else {
                            "↺ Refresh Devices"
                        }),
                    )
                    .clicked()
                {
                    spawn_fetch_devices(bearer.clone(), app_id, &mut state, &mut discovery_task);
                }
            }

            // Device picker dropdown, note we show the name not the id maybe I
            // should show id in a pop up?
            // Auto-select the only device when there is exactly one choice.
            if state.devices.len() == 1 && state.selected_device.is_none() {
                state.selected_device = Some(0);
            }

            if !state.devices.is_empty() {
                ui.add_space(4.0);
                ui.label("Device:");
                let dev_label = state
                    .selected_device
                    .and_then(|i| state.devices.get(i))
                    .map(|d| d.name.as_str())
                    .unwrap_or("select device");

                let dev_names: Vec<(usize, String)> = state
                    .devices
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (i, d.name.clone()))
                    .collect();
                let mut new_dev_sel = None;
                egui::ComboBox::from_id_salt("losant_device_picker")
                    .selected_text(dev_label)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for (idx, name) in &dev_names {
                            let selected = state.selected_device == Some(*idx);
                            if ui.selectable_label(selected, name).clicked() {
                                new_dev_sel = Some(*idx);
                            }
                        }
                    });
                if let Some(idx) = new_dev_sel {
                    state.selected_device = Some(idx);
                }
            }

            // Raw SSE replies, note the underlying losant DeqQueue only keeps
            // the last 100 so this doesn't get too obscene.
            let ready_to_stream =
                state.selected_application.is_some() && state.selected_device.is_some();

            if ready_to_stream {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Raw Device State Stream").strong());
                ui.separator();
                ui.add_space(4.0);

                let connected = matches!(
                    state.sse_status,
                    LosantSseStatus::Connecting | LosantSseStatus::Connected
                );

                ui.horizontal(|ui| {
                    if connected {
                        if ui.button("⏹ Disconnect").clicked() {
                            spawn_sse_disconnect(&mut state, &mut sse_task, &mut sse_channel);
                        }
                    } else if ui.button("▶ Connect").clicked() {
                        let app_id = state
                            .selected_application
                            .and_then(|i| state.applications.get(i))
                            .map(|a| a.id.clone())
                            .unwrap_or_default();
                        let device_id = state
                            .selected_device
                            .and_then(|i| state.devices.get(i))
                            .map(|d| d.id.clone())
                            .unwrap_or_default();
                        spawn_sse_connect(
                            bearer.clone(),
                            app_id,
                            device_id,
                            &mut state,
                            &mut sse_task,
                            &mut sse_channel,
                        );
                    }

                    let (status_text, status_color) = match &state.sse_status {
                        LosantSseStatus::Disconnected => {
                            ("Disconnected".to_string(), egui::Color32::GRAY)
                        }
                        LosantSseStatus::Connecting => (
                            "⏳ Connecting hold please".to_string(),
                            egui::Color32::YELLOW,
                        ),
                        LosantSseStatus::Connected => (
                            "● Connected".to_string(),
                            egui::Color32::from_rgb(80, 200, 120),
                        ),
                        LosantSseStatus::Error(msg) => {
                            (format!("✖ {msg}"), egui::Color32::from_rgb(220, 80, 80))
                        }
                    };
                    ui.label(egui::RichText::new(status_text).color(status_color));
                });

                ui.add_space(4.0);

                // Raw Event log sorted by newest at top, show latest 20 for now
                // cause 100 is a lot of scrolling.
                let log_height = 160.0;
                egui::ScrollArea::vertical()
                    .id_salt("losant_event_log")
                    .max_height(log_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if state.event_log.is_empty() {
                            ui.label(
                                egui::RichText::new("No events received so far.")
                                    .weak()
                                    .italics(),
                            );
                        } else {
                            for entry in state.event_log.iter().take(20) {
                                ui.label(egui::RichText::new(entry).small().monospace());
                                ui.separator();
                            }
                        }
                    });
            }
        });

    if !open && let Ok(entity) = losant_query.single() {
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
    let Some(engine) = engine else {
        ui.label(egui::RichText::new("No model loaded.").weak().italics());
        return;
    };

    if result.matches.is_empty() {
        ui.label(
            egui::RichText::new("Draw something\nto see results.")
                .weak()
                .italics(),
        );
        return;
    }

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

/// System to populate `CachedAdapterInfo` from `RenderAdapterInfo` the first time the
/// render world makes it available to callers. Only runs while `CachedAdapterInfo` is
/// unpopulated, only runs once as the wgpu backend cannot change at runtime afaik.
fn cache_adapter_info(
    adapter_info: Option<Res<RenderAdapterInfo>>,
    mut cached: ResMut<CachedAdapterInfo>,
) {
    if let Some(info) = adapter_info {
        cached.0 = Some(format!("{:?} ({})", info.backend, info.name));
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
    egui_wants_input.wants_pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui();
    egui_wants_input.wants_keyboard = ctx.egui_wants_keyboard_input();

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
            &mut crate::plugins::fullscreen::CameraOrbit,
            &crate::plugins::camera::FreeLookCamera,
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
            &mut crate::plugins::fullscreen::CameraOrbit,
            &mut crate::plugins::camera::FreeLookCamera,
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

        let defaults = crate::plugins::fullscreen::CameraConfig::default();

        *transform = defaults.transform;
        *orbit = defaults.orbit;
        *free_look = defaults.free_look;
        *projection = Projection::Perspective(PerspectiveProjection::default());
        *camera_mode = CameraMode::Perspective;

        bevy::log::debug!("user reset camera to default");
    }
}

pub fn apply_pending_app_switch(
    mut pending: ResMut<PendingAppSwitch>,
    mut active_app: ResMut<ActiveApp>,
) {
    if let Some(next) = pending.0.take()
        && *active_app != next
    {
        *active_app = next;
        bevy::log::debug!("ActiveApp switched to {:?}", *active_app);
    }
}
