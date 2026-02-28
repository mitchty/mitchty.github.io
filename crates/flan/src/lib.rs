use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

/// Maximum number of plot points stored in the WebGL2 uniform buffer.
/// Must match the array size in the WESL shader source (`array<vec4<f32>, 512>`).
pub const MAX_PLOT_POINTS: usize = 512;

/// Marker component so the animation system can find the spawned plot UI node.
#[derive(Component)]
pub struct PlotUiNode;

/// Number of data points kept in the scrolling plot buffer (for now....)
/// Oldest point is at the front idx 0, newest at the limit
pub const PLOT_BUFFER_SIZE: usize = 200;

/// Circular scrolling buffer for the plot shader. Here so I can add "new" data
/// every second and it looks like this is plotting... something. The shader
/// always gets from oldest to newest so it "looks" like its animating rightward
/// with new data.
#[derive(Resource)]
pub struct PlotDataState {
    pub ys: VecDeque<f32>,
}

pub struct PlotPlugin;

impl Plugin for PlotPlugin {
    fn build(&self, app: &mut App) {
        // ShadersPlugin must already have been added by the consuming app
        // mitchty adds it in main before PlotPlugin for now.
        app.add_plugins(Material2dPlugin::<PlotMaterial>::default())
            .add_plugins(UiMaterialPlugin::<PlotUiMaterial>::default())
            .add_systems(Startup, setup_plot_ui)
            .add_systems(
                Update,
                (
                    animate_plot_time,
                    update_plot_data.run_if(bevy::time::common_conditions::on_timer(
                        std::time::Duration::from_millis(379),
                    )),
                ),
            );
    }
}

fn setup_plot_ui(
    mut commands: Commands,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut y = rng.random_range(0.0f32..=1.0f32);

    let ys: VecDeque<f32> = (0..PLOT_BUFFER_SIZE)
        .map(|_| {
            let step: f32 = rng.random_range(-0.1..=0.1);
            y = (y + step).clamp(0.0, 1.0);
            y
        })
        .collect();

    let points = ys_to_points(&ys);

    commands.insert_resource(PlotDataState { ys });

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
            // Keep this in the middle vertically
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

/// Converts the deque yvals into a `Vec<Vec2>` with X in range of [0, 1] for
/// the shader uv code to be simple.
fn ys_to_points(ys: &VecDeque<f32>) -> Vec<Vec2> {
    let n = ys.len();
    ys.iter()
        .enumerate()
        .map(|(i, &y)| Vec2::new(i as f32 / (n - 1) as f32, y))
        .collect()
}

/// Updates the circular buffer one step data point to the right and drops the
/// oldest Y from the deque head, appends one new Y bounded to not too far from
/// the latest value to the deque tail, then uploads the full buffer to the
/// shader. Basically every tick of this system we gain a new element to what
/// the plot shader contains.
fn update_plot_data(
    mut state: ResMut<PlotDataState>,
    node_query: Query<&MaterialNode<PlotUiMaterial>, With<PlotUiNode>>,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    use rand::Rng;
    let mut rng = rand::rng();

    state.ys.pop_front();
    let last = *state.ys.back().unwrap_or(&0.5);
    let step: f32 = rng.random_range(-0.1..=0.1);
    state.ys.push_back((last + step).clamp(0.0, 1.0));

    let points = ys_to_points(&state.ys);

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
/// `time` is elapsed seconds — increment it every frame to drive shader
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
/// layout agree exactly under std140 (WebGL2 GLSL uniform block rules).
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
        #[cfg(not(feature = "webgl"))]
        {
            shaders::BEVY_DEFAULT_MATERIAL_PLOT.clone().into()
        }
        #[cfg(feature = "webgl")]
        {
            shaders::BEVY_WEBGL_MATERIAL_PLOT.clone().into()
        }
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// UI Material version of the plot shader — rendered as a Bevy UI node.
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
        #[cfg(not(feature = "webgl"))]
        {
            shaders::BEVY_DEFAULT_UI_PLOT.clone().into()
        }
        #[cfg(feature = "webgl")]
        {
            shaders::BEVY_WEBGL_UI_PLOT.clone().into()
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
        // WebGL requires uniform buffer structs to be multiples of 16 bytes
        assert_eq!(
            size_of::<PlotPointsUniform>() % 16,
            0,
            "PlotPointsUniform must be a multiple of 16 bytes for WebGL (got {} bytes)",
            size_of::<PlotPointsUniform>()
        );
    }

    #[test]
    fn max_plot_points_constant() {
        // MAX_PLOT_POINTS must match the array size in the WESL shader
        // (array<vec4<f32>, 512>). If you change this, update the shader too!
        assert_eq!(MAX_PLOT_POINTS, 512);
    }
}
