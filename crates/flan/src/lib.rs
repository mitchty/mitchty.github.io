use bevy::prelude::UiMaterialKey;
use bevy::prelude::*;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::render_resource::*;
use bevy::shader::ShaderDefVal;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use polars::prelude::*;

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

/// Maximum number of plot points stored in the WebGL2 uniform buffer.
/// Must match the array size in the WESL shader source `array<vec4<f32>, 512>`.
pub const MAX_PLOT_POINTS: usize = 512;

/// Marker component so the animation system can find the spawned plot UI node.
#[derive(Component)]
pub struct PlotUiNode;

/// How many of the most-recent DataFrame rows are windowed and uploaded to the
/// shader each sync. The DataFrame itself can be arbitrarily large; only the
/// tail `PLOT_WINDOW_SIZE` rows are ever sent to the GPU.
pub const PLOT_WINDOW_SIZE: usize = 200;

/// The backing polars DataFrame that owns all plot data.
///
/// Callers own this resource and mutate it directly.  After
/// mutating, fire a `PlotDataUpdated` event and `flan` will sync the newest
/// `PLOT_WINDOW_SIZE` rows to the shader automatically.
///
/// Schema: must contain at least a column named `"y"` of dtype `Float32`.
/// X coordinates are derived by `flan` from the row's position within the
/// window. Callers never need to store or update them.
#[derive(Resource)]
pub struct PlotDataFrame {
    pub df: DataFrame,
}

/// Send this message after mutating `PlotDataFrame` to tell `flan` to
/// re-sync the shader buffer. `flan`s `sync_plot_data` system fires on this
/// message and uploads the last `PLOT_WINDOW_SIZE` rows from the `"y"` column.
// TODO: make the column selectable at some point so I can make this more
// dynamic or whatever. This is a classic "FUTURE MITCH" problem past mitch is
// punting on. Suck it future me.
pub struct PlotDataUpdated;
impl bevy::ecs::message::Message for PlotDataUpdated {}

pub struct PlotPlugin;

impl Plugin for PlotPlugin {
    fn build(&self, app: &mut App) {
        // ShadersPlugin must already have been added by the consuming app.
        // mitchty adds it in main before PlotPlugin for now.
        //
        // PlotDataFrame is NOT inserted here callers are
        // responsible for inserting that data before Startup runs so that
        // setup_plot_ui can do the initial upload.  If no PlotDataFrame exists
        // at startup an empty shader buffer is used so no plot for you sucker.
        app.add_plugins(Material2dPlugin::<PlotMaterial>::default())
            .add_plugins(UiMaterialPlugin::<PlotUiMaterial>::default())
            .add_message::<PlotDataUpdated>()
            .add_systems(Startup, setup_plot_ui)
            .add_systems(
                Update,
                (
                    animate_plot_time,
                    // Sync whenever the caller sends PlotDataUpdated to
                    // whatever is in the underlying dataframe.
                    sync_plot_data.run_if(
                        bevy::ecs::schedule::common_conditions::on_message::<PlotDataUpdated>,
                    ),
                ),
            );
    }
}

fn setup_plot_ui(
    mut commands: Commands,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    plot_df: Option<Res<PlotDataFrame>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    // Iff the dataframes empty, the points are too (all 0's basically, good
    // luck plotting nothing)
    let points = plot_df
        .as_ref()
        .map(|r| df_tail_to_points(&r.df))
        .unwrap_or_default();

    #[cfg(not(feature = "webgl"))]
    let points_binding = buffers.add(ShaderStorageBuffer::from(points.clone()));

    #[cfg(feature = "webgl")]
    let points_binding = {
        let mut data = [Vec4::ZERO; MAX_PLOT_POINTS];
        for (i, p) in points.iter().enumerate().take(MAX_PLOT_POINTS) {
            data[i] = Vec4::new(p.x, p.y, 0.0, 0.0);
        }
        PlotPointsUniform { data }
    };

    let material = ui_materials.add(PlotUiMaterial {
        params: PlotUniform {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            zoom: Vec2::ONE,
            offset: Vec2::ZERO,
            count: points.len().min(MAX_PLOT_POINTS) as u32,
            time: 0.0,
            #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
            _webgl2_padding: Vec2::ZERO,
        },
        points: points_binding,
    });

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            // Keep this vertically centred: top 50% minus half the widget height.
            top: Val::Percent(50.0),
            margin: UiRect {
                top: Val::Px(-100.0),
                ..default()
            },
            width: Val::Px(200.0),
            height: Val::Px(200.0),
            ..default()
        },
        MaterialNode(material),
        PlotUiNode,
    ));
}

/// Slice the tail `PLOT_WINDOW_SIZE` rows of the `"y"` column from a
/// DataFrame and convert them to `Vec<Vec2>` with X ∈ [0, 1].
///
/// Rows are mapped oldest to newest left to right so the shader sees the expected
/// scrolling-timeline layout.  Returns an empty Vec if the column is missing
/// or the DataFrame is empty.
fn df_tail_to_points(df: &DataFrame) -> Vec<Vec2> {
    let Ok(series) = df.column("y") else {
        return Vec::new();
    };
    let ca = series.cast(&DataType::Float32).ok();
    let ca = ca.as_ref().and_then(|s| s.f32().ok());
    let Some(ca) = ca else {
        return Vec::new();
    };

    // Window to the last PLOT_WINDOW_SIZE values.
    let total = ca.len();
    let start = total.saturating_sub(PLOT_WINDOW_SIZE);
    let slice = ca.slice(start as i64, PLOT_WINDOW_SIZE);

    let n = slice.len();
    if n == 0 {
        return Vec::new();
    }
    let denom = (n - 1).max(1) as f32;
    slice
        .into_iter()
        .enumerate()
        .map(|(i, y)| Vec2::new(i as f32 / denom, y.unwrap_or(0.0)))
        .collect()
}

/// Triggered by `PlotDataUpdated` events fired by the caller.
///
/// Reads the last `PLOT_WINDOW_SIZE` rows from the `"y"` column of
/// `PlotDataFrame` and uploads them to every spawned plot UI node's shader
/// buffer.  If `PlotDataFrame` has not been inserted this system is a no-op.
fn sync_plot_data(
    plot_df: Option<Res<PlotDataFrame>>,
    node_query: Query<&MaterialNode<PlotUiMaterial>, With<PlotUiNode>>,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let Some(plot_df) = plot_df else { return };
    let points = df_tail_to_points(&plot_df.df);

    for material_node in node_query.iter() {
        let Some(mat) = ui_materials.get_mut(material_node) else {
            continue;
        };

        #[cfg(not(feature = "webgl"))]
        if let Some(buf) = buffers.get_mut(&mat.points) {
            *buf = ShaderStorageBuffer::from(points.clone());
        }

        #[cfg(feature = "webgl")]
        {
            let mut data = [Vec4::ZERO; MAX_PLOT_POINTS];
            for (i, p) in points.iter().enumerate().take(MAX_PLOT_POINTS) {
                data[i] = Vec4::new(p.x, p.y, 0.0, 0.0);
            }
            mat.points = PlotPointsUniform { data };
            mat.params.count = points.len().min(MAX_PLOT_POINTS) as u32;
        }
    }
}

/// Keep `params.time` ticking so downstream code / future shaders can use it
/// for time based changes to displaying data.
///
/// Here for future work where time might be used for something in the shaders
/// different from the actual data shown.
fn animate_plot_time(
    time: Res<Time>,
    node_query: Query<&MaterialNode<PlotUiMaterial>, With<PlotUiNode>>,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
) {
    for material_node in node_query.iter() {
        if let Some(mat) = ui_materials.get_mut(material_node) {
            mat.params.time = time.elapsed_secs();
        }
    }
}

/// Uniform data shared between the Rust side and every plot shader variant.
///
/// `count` is only consumed by the WEBGL path (the storage-buffer path uses
/// `arrayLength` instead), but it is always present so the WGSL struct layout
/// is identical across all variants.
///
/// `time` is elapsed seconds; increment it every frame to drive shader
/// animation (e.g. a travelling sin-wave phase offset).
#[derive(Clone, Copy, ShaderType)]
pub struct PlotUniform {
    pub min: Vec2,
    pub max: Vec2,
    pub zoom: Vec2,
    pub offset: Vec2,
    /// Number of valid points in the `points` array.  Set this to the actual
    /// point count on every update; ignored on non-WebGL builds at runtime.
    pub count: u32,
    /// Elapsed time in seconds.  Upload `time.elapsed_secs()` here every
    /// frame so the shader can animate.
    pub time: f32,
    #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
    pub _webgl2_padding: Vec2,
}

/// Points buffer used in WebGL2 builds (uniform buffer, WEBGL feature).
///
/// Uses `Vec4` so that the Rust layout and the WGSL `array<vec4<f32>, 512>`
/// layout agree exactly under std140 WebGL2 GLSL uniform block rules.
/// The `.xy` components hold the actual `(x, y)` data; `.zw` are unused.
///
/// Must match `MAX_PLOT_POINTS` and the WESL shader constant.
#[cfg(feature = "webgl")]
#[derive(Clone, ShaderType)]
pub struct PlotPointsUniform {
    pub data: [Vec4; MAX_PLOT_POINTS],
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct PlotMaterial {
    #[uniform(0)]
    pub params: PlotUniform,

    /// Storage buffer (native / WebGPU).
    #[cfg(not(feature = "webgl"))]
    #[storage(1, read_only)]
    pub points: Handle<ShaderStorageBuffer>,

    /// Uniform buffer (WebGL2).
    #[cfg(feature = "webgl")]
    #[uniform(1)]
    pub points: PlotPointsUniform,
}

impl Material2d for PlotMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://shaders/2d/plot.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let webgl = cfg!(all(
            feature = "webgl",
            target_arch = "wasm32",
            not(feature = "webgpu")
        ));
        if let Some(ref mut fragment) = descriptor.fragment {
            fragment
                .shader_defs
                .push(ShaderDefVal::Bool("WEBGL".into(), webgl));
        }
        let _ = layout;
        Ok(())
    }
}

/// UI Material version of the plot shader rendered as a Bevy UI node.
///
/// Binding layout mirrors [`PlotMaterial`] but targets `@group(1)` which is
/// what Bevy's `UiMaterial` pipeline uses.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct PlotUiMaterial {
    #[uniform(0)]
    pub params: PlotUniform,

    /// Storage buffer (native / WebGPU).
    #[cfg(not(feature = "webgl"))]
    #[storage(1, read_only)]
    pub points: Handle<ShaderStorageBuffer>,

    /// Uniform buffer (WebGL2).
    #[cfg(feature = "webgl")]
    #[uniform(1)]
    pub points: PlotPointsUniform,
}

impl UiMaterial for PlotUiMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://shaders/2d/plot.wesl".into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        let webgl = cfg!(all(
            feature = "webgl",
            target_arch = "wasm32",
            not(feature = "webgpu")
        ));
        if let Some(ref mut fragment) = descriptor.fragment {
            fragment
                .shader_defs
                .push(ShaderDefVal::Bool("WEBGL".into(), webgl));
            // Ensure the shader uses @group(1) bindings
            fragment
                .shader_defs
                .push(ShaderDefVal::Bool("UI_MATERIAL".into(), true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
    fn plot_uniform_webgl_alignment() {
        use std::mem::size_of;
        // WebGL requires uniform buffer structs to be multiples of 16 bytes
        assert_eq!(
            size_of::<PlotUniform>() % 16,
            0,
            "PlotUniform must be a multiple of 16 bytes for WebGL (got {} bytes)",
            size_of::<PlotUniform>()
        );
    }

    #[test]
    #[cfg(feature = "webgl")]
    fn plot_points_uniform_webgl_alignment() {
        use std::mem::size_of;
        assert_eq!(
            size_of::<PlotPointsUniform>() % 16,
            0,
            "PlotPointsUniform must be a multiple of 16 bytes for WebGL (got {} bytes)",
            size_of::<PlotPointsUniform>()
        );
    }

    #[test]
    // TODO:         LEAK [   0.229s] ( 7/22) flan tests::max_plot_points_constant
    // future me can figure out what the hell could possibly leak here
    fn max_plot_points_constant() {
        // MAX_PLOT_POINTS must match the array size in the WESL shader
        // array<vec4<f32>, 512>. If you change this, update the shader too dumdum!
        assert_eq!(MAX_PLOT_POINTS, 512);
    }
}
