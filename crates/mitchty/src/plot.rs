use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;
use flan::{MAX_PLOT_POINTS, PlotPlugin, PlotUniform};

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

#[cfg(feature = "webgl")]
use flan::PlotPointsUniform;

pub struct PlotSetupPlugin;

impl Plugin for PlotSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PlotPlugin)
            .add_plugins(UiMaterialPlugin::<PlotUiMaterial>::default())
            .add_systems(Startup, setup_plot_ui);
    }
}

/// UI Material version of the plot shader — rendered as a Bevy UI node.
/// Binding layout mirrors PlotMaterial in flan but uses @group(1) (UiMaterial).
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

fn setup_plot_ui(
    mut commands: Commands,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let points: Vec<Vec2> = (0..200)
        .map(|i| {
            let x = i as f32 / 199.0;
            let y = (x * 10.0).sin() * 0.4 + 0.5;
            Vec2::new(x, y)
        })
        .collect();

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
            #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
            _webgl2_padding: Vec2::ZERO,
        },
        points: points_binding,
    });

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(10.0),
            width: Val::Px(200.0),
            height: Val::Px(200.0),
            ..default()
        },
        MaterialNode(material),
    ));
}
