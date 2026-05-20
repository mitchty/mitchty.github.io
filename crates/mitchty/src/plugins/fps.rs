//! FPS display, sparkline, and plot-data plugins and related shenanigans.
//!
//! Owns the FPS text overlay, the sparkline UI node, the `FpsHistory`
//! circular buffer, and the random-walk `PlotDataFrame` that feeds the flan
//! plot shader.
// TODO: remove the stupid flan 2d plot shader its a bit of a human tail of me experimenting

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use polars::prelude::*;
use rand::RngExt;

/// Marker component for the FPS text entity.
#[derive(Component)]
pub struct FpsText;

/// Marker component: while present the FPS readout and its requisite sparkline
/// are visible this controls both ui elements.
#[derive(Component, Default)]
pub struct FpsDisplay;

/// Number of FPS samples retained - 100 x 100 ms ≈ 10 s of history.
pub const FPS_HISTORY_SAMPLES: usize = 100;

/// Preallocated circular buffer of recent FPS measurements.
///
/// Slots start as `None` and fill in as samples arrive. Once full, the oldest
/// sample is overwritten so things act as a poor mans circular buffer.
#[derive(Resource)]
pub struct FpsHistory {
    pub data: [Option<f32>; FPS_HISTORY_SAMPLES],
    /// Index of the *next* slot to write into.
    pub index: usize,
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
    /// Write `fps` into the current slot and advance the write cursor.
    pub fn push(&mut self, fps: f32) {
        self.data[self.index] = Some(fps);
        self.index = (self.index + 1) % FPS_HISTORY_SAMPLES;
    }

    /// Return filled samples in chronological order, normalized so the maximum
    /// observed value maps to `1.0`.
    pub fn to_normalized_values(&self) -> Vec<f32> {
        let raw: Vec<f32> = (0..FPS_HISTORY_SAMPLES)
            .map(|offset| (self.index + offset) % FPS_HISTORY_SAMPLES)
            .filter_map(|slot| self.data[slot])
            .collect();

        if raw.is_empty() {
            return Vec::new();
        }

        let max_fps = self
            .data
            .iter()
            .filter_map(|v| *v)
            .fold(f32::NEG_INFINITY, f32::max);

        let scale = if max_fps > 0.0 { max_fps } else { 1.0 };

        // Invert y so higher FPS is rendered lower on the sparkline strip.
        raw.iter()
            .map(|&fps| 1.0 - (fps / scale).clamp(0.0, 1.0))
            .collect()
    }
}

/// Startup system to spawn the FPS text node in the upper-right corner.
pub fn setup_fps_ui(mut commands: Commands, ui_state: Res<crate::ui::state::UiState>) {
    let top = if ui_state.backend == crate::ui::state::UiBackend::Egui {
        Val::Px(40.0)
    } else {
        Val::Px(10.0)
    };

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

/// Startup system to spawn the FPS sparkline UI node.
pub fn setup_fps_sparkline_ui(
    mut commands: Commands,
    mut ui_materials: ResMut<Assets<flan::PlotUiMaterial>>,
    ui_state: Res<crate::ui::state::UiState>,
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
        },
        points: points_binding,
    });

    let top = if ui_state.backend == crate::ui::state::UiBackend::Egui {
        Val::Px(40.0)
    } else {
        Val::Px(10.0)
    };

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
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

/// Update FPS text and sparkline visibility.
///
/// Runs every 500 ms via `on_timer` so it doesn't look weird af, 500ms is
/// probably too fast as it is.
// TODO: future mitch maybe make the display timer something that can be configured at runtime dynamically?
pub fn update_fps_ui(
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

/// Sample the current FPS into `FpsHistory` roughly every 100 ms.
pub fn sample_fps_history(
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
    sparkline_df.df = DataFrame::new(n, vec![Column::new("y".into(), values)]).unwrap_or_default();
}

/// Toggle `FpsDisplay` marker on/off with the `F` key.
pub fn toggle_fps_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    mut commands: Commands,
    #[cfg(feature = "egui")] egui_wants_input: Res<crate::ui::EguiWantsInput>,
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

// TODO: The plot and dataframe stuff is likely better located in the lib crate.

/// Build the initial random-walk `DataFrame` used by the flan plot shader.
pub fn initial_plot_df() -> DataFrame {
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

/// Append a new random-walk row to the plot `DataFrame` and cap at
/// `PLOT_WINDOW_SIZE` rows.
pub fn tick_plot_data(
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

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FpsHistory>()
            .insert_resource(flan::PlotDataFrame {
                df: initial_plot_df(),
            })
            .insert_resource(flan::SparklineDataFrame {
                df: DataFrame::empty(),
            })
            .add_systems(Startup, (setup_fps_ui, setup_fps_sparkline_ui))
            .add_systems(
                Update,
                update_fps_ui.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_millis(500),
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
                tick_plot_data.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_millis(100),
                )),
            )
            .add_systems(Update, toggle_fps_display);
    }
}
