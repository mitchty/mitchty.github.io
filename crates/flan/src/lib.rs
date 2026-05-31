use bevy::pbr::Material;
use bevy::prelude::UiMaterialKey;
use bevy::prelude::*;
use bevy::render::alpha::AlphaMode;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::render_resource::*;
use bevy::shader::ShaderDefVal;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use polars::prelude::*;

#[cfg(not(feature = "webgl"))]
use bevy::render::storage::ShaderStorageBuffer;

pub mod layout;
pub mod shaders;
pub mod slug_text;

pub use layout::{Horizontal, Layout, Vertical};
pub use slug_text::{SlugPlugin, SlugTextFont, SlugTextMesh, SlugTextNode};

#[cfg(all(feature = "render", not(target_arch = "wasm32")))]
pub mod render;
#[cfg(all(feature = "render", not(target_arch = "wasm32")))]
pub mod snapshot;
#[cfg(all(feature = "render", not(target_arch = "wasm32")))]
pub mod wesl;

/// Slug font atlas builder is mostly TTF/OTF curve datato GPU curve/band
/// buffers for the Slug renderer. Uses ttf-parser and bytemuck for the evil.
pub mod slug;

/// Glyph extrusion to build side-wall geometry from flan slug bezier contours
/// to replace bevy fontmeash usage so I don't have holes all over in kanji glyphs.
pub mod extrude;
pub use extrude::build_text_side_walls;

/// Maximum number of plot points stored in a webgl uniform buffer. Must match
/// the array size in the WESL shader source `array<vec4<f32>, 512>`.
// TODO: I wonder if I could sync this magic number in build.rs to build a
// constants.rs/constants.wesl instead of trying to remember to keep crap in
// sync which I never do. I cannnnnnnnoot wait until webgpu works so I can ditch
// this webgl bs.
pub const MAX_PLOT_POINTS: usize = 512;

/// Marker component so the animation system can find the spawned plot UI node.
#[derive(Component)]
pub struct PlotUiNode;

/// Marker component for an fps sparkline plot, abusing flan for this idea to
/// benchmark crap.
///
/// Callers spawn a `MaterialNode<PlotUiMaterial>` with this component attached
/// and place it wherever they like. `flan` will sync data from
/// [`SparklineDataFrame`] to every entity carrying this marker. All we do here
/// is populate the dataframe with data.
#[derive(Component)]
pub struct SparklineUiNode;

/// How many of the most-recent DataFrame rows are windowed and uploaded to the
/// shader each sync. The DataFrame itself can be arbitrarily large; only the
/// tail `PLOT_WINDOW_SIZE` rows are ever sent to the GPU.
pub const PLOT_WINDOW_SIZE: usize = 200;

/// The backing polars DataFrame that owns all plot data.
///
/// Callers own this resource and mutate it directly. After
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

/// Backing DataFrame for the sparkline strip widget which for now is fps.
/// If/when I do more future me problem on making a more complect approach.
///
/// Schema is a single `"y"` column of `Float32` values already normalized to
/// `[0, 1]` by the caller. Null / `None` values are skipped during upload so
/// the sparkline only draws the portion of the history that has real data so
/// the plot doesn't look stupid with edge cases that likely aren't relevant.
// TODO: This is all hold my beer approach. I think I need a plugin for flan to
// cover how it can handle dataframes for us or something.
#[derive(Resource)]
pub struct SparklineDataFrame {
    pub df: DataFrame,
}

pub struct PlotPlugin;

impl Plugin for PlotPlugin {
    fn build(&self, app: &mut App) {
        // ShadersPlugin must already have been added by the consuming app.
        // mitchty adds it in main before PlotPlugin for now.
        //
        // PlotDataFrame is NOT inserted here callers are
        // responsible for inserting that data before Startup runs so that
        // setup_plot_ui can do the initial upload. If no PlotDataFrame exists
        // at startup an empty shader buffer is used so no plot for you sucker.
        app.add_plugins(Material2dPlugin::<PlotMaterial>::default())
            .add_plugins(UiMaterialPlugin::<PlotUiMaterial>::default())
            .add_message::<PlotDataUpdated>()
            .add_systems(Startup, setup_plot_ui)
            .add_systems(
                Update,
                (
                    animate_plot_time,
                    // Sync whenever the caller sends PlotDataUpdated.
                    sync_plot_data.run_if(
                        bevy::ecs::schedule::common_conditions::on_message::<PlotDataUpdated>,
                    ),
                    // Sync sparkline df whenever SparklineDataFrame is mutated.
                    sync_sparkline_data.run_if(resource_changed::<SparklineDataFrame>),
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
            line_width: 0.003, // TODO: I should make this dynamic somehow future me problem
        },
        points: points_binding,
    });

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            // Keep this vertically centered: top 50% minus half the widget height.
            top: Val::Percent(50.0),
            margin: UiRect {
                top: Val::Px(-100.0),
                ..default()
            },
            width: Val::Px(200.0),
            height: Val::Px(200.0),
            ..default()
        },
        Visibility::Hidden,
        MaterialNode(material),
        PlotUiNode,
    ));
}

/// Triggered by Bevy change detection whenever [`SparklineDataFrame`] is mutated.
///
/// Reads the `"y"` column, filters null rows, and uploads to every
/// [`SparklineUiNode`] entity's shader buffer. Y values must already be
/// normalized to `[0, 1]` by the caller.
fn sync_sparkline_data(
    sparkline_df: Res<SparklineDataFrame>,
    node_query: Query<&MaterialNode<PlotUiMaterial>, With<SparklineUiNode>>,
    mut ui_materials: ResMut<Assets<PlotUiMaterial>>,
    #[cfg(not(feature = "webgl"))] mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    // Reuse the shared helper to strip nulls from the source dataframe.
    let points = df_to_points(&sparkline_df.df);

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
        }

        mat.params.count = points.len().min(MAX_PLOT_POINTS) as u32;
    }
}

/// Convert the `"y"` column of a DataFrame to `Vec<Vec2>` with X ∈ [0, 1].
///
/// Rows are mapped from oldest to newest, left to right. Null values are
/// ignored so callers can use null rows as sentinels for "not yet filled" slots
/// for... god knows why right now its mostly to not have the plot go weird for
/// no reason. Its just an fps plot its not critical it be perfect. Returns an
/// empty Vec if the column is missing or the DataFrame is empty which means
/// likely a caller won't show anything.
fn df_to_points(df: &DataFrame) -> Vec<Vec2> {
    let Ok(series) = df.column("y") else {
        return Vec::new();
    };
    let ca = series.cast(&DataType::Float32).ok();
    let ca = ca.as_ref().and_then(|s| s.f32().ok());
    let Some(ca) = ca else {
        return Vec::new();
    };

    // Collect only non-null values, I can't see what purpose it would be to use
    // them in a plot like this yet.
    let values: Vec<f32> = ca.into_iter().flatten().collect();

    // Bail early if there is nothing to show
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }

    let denom = (n - 1).max(1) as f32;
    values
        .into_iter()
        .enumerate()
        .map(|(i, y)| Vec2::new(i as f32 / denom, y))
        .collect()
}

/// Slice the tail `PLOT_WINDOW_SIZE` rows then delegate to [`df_to_points`].
///
/// Used by the main line-graph plot so the DataFrame the shader uses only sees
/// the most recent window.
fn df_tail_to_points(df: &DataFrame) -> Vec<Vec2> {
    let Ok(series) = df.column("y") else {
        return Vec::new();
    };
    let ca = series.cast(&DataType::Float32).ok();
    let ca = ca.as_ref().and_then(|s| s.f32().ok());
    let Some(ca) = ca else {
        return Vec::new();
    };

    let total = ca.len();
    let start = total.saturating_sub(PLOT_WINDOW_SIZE);
    let sliced_series = ca
        .slice(start as i64, PLOT_WINDOW_SIZE)
        .into_series()
        .with_name("y".into());
    let windowed =
        DataFrame::new(sliced_series.len(), vec![Column::from(sliced_series)]).unwrap_or_default();
    df_to_points(&windowed)
}

/// Triggered by `PlotDataUpdated` events fired by the caller.
///
/// Reads the last `PLOT_WINDOW_SIZE` rows from the `"y"` column of
/// `PlotDataFrame` and uploads them to every spawned plot UI node's shader
/// buffer. If `PlotDataFrame` has not been inserted this system is a no-op.
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
    /// Number of valid points in the `points` array. Set this to the actual
    /// point count on every update; ignored on non-webgl builds at runtime.
    pub count: u32,
    /// Elapsed time in seconds. Upload `time.elapsed_secs()` here every
    /// frame so the shader can animate.
    pub time: f32,
    /// Antialiased half-width of the polyline in UV space.
    /// #[shader(size(8))]: pads this field to 8 bytes so the struct is 48
    /// bytes total (4xvec2 + u32 + f32 + f32 = 44 -> rounded to 48),
    /// satisfying webgl's requirement that uniform bindings are multiples of 16.
    #[shader(size(8))]
    pub line_width: f32,
}

/// Points buffer used in webgl builds (uniform buffer, webgl feature).
///
/// Uses `Vec4` so that the Rust layout and the WGSL `array<vec4<f32>, 512>`
/// layout agree exactly under std140 webgl GLSL uniform block rules.
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

    #[cfg(feature = "webgl")]
    #[uniform(1)]
    pub points: PlotPointsUniform,
}

impl Material2d for PlotMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/2d/plot.wesl".into()
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

    #[cfg(feature = "webgl")]
    #[uniform(1)]
    pub points: PlotPointsUniform,
}

impl UiMaterial for PlotUiMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/2d/plot.wesl".into()
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

// TODO: I'm copypastaing a lot of crap between wesl and rust. I should yeet
// this into build.rs and have a shared consts file betwixt ye olde rust lande
// and ye weirde gpu lande and the source of truth is always rust.
//
// Texture width for webgl data textures. Must match SLUG_TEX_WIDTH in
// lib/slug/types.wesl.
pub const SLUG_TEX_WIDTH: u32 = 2048;

// TODO: All this crap too is a good candidate for a refactor to keep the params
// et al in sync.
/// SlugParams uniform 16 bytes UiMaterial-only junk in the trunk to mimic bevy ui.
///
/// Layout must match `SlugParams` in lib/slug/types.wesl, future mitch combine stop lazy:
///   0   node_size    : `vec2<f32>`  - resolved node pixel size; used by ui_text.wesl
///                                     to convert in.uv -> pixel coordinates before
///                                     calling slugtext().
///   8   layout_flags : u32          - 4-bit packed Layout bitfield (see layout.rs).
///                                     Passed directly to slugtext() in ui_text.wesl.
///   12  _pad         : u32
#[derive(Clone, Copy, Default, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SlugParams {
    pub node_size: Vec2,
    pub layout_flags: u32,
    pub _pad: u32,
}

/// Per-run descriptor uploaded once per shaped text string.
///
/// 16 bytes = 1 x `vec4<f32>` fits in a single rgba32float texel on webgl.
///
/// Layout must match `SlugRunDesc` in lib/slug/types.wesl:
///   0   natural_advance : f32  - total advance width of the run at font_size (px)
///   4   natural_height  : f32  - (ascender − descender) x scale at font_size (px)
///   8   glyph_offset    : u32  - first index into glyph_layout[] for this run
///   12  glyph_count     : u32  - number of SlugGlyphLayout entries in this run
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SlugRunDesc {
    pub natural_advance: f32,
    pub natural_height: f32,
    pub glyph_offset: u32,
    pub glyph_count: u32,
}

/// Packed atlas layout produced by `SlugAtlas::build_frame_atlas`.
/// Stores byte offsets and sizes into a single contiguous buffer on native,
/// or the dimensions needed to build webgl data textures for effing webgl's bs.
#[derive(Clone, Default, Debug)]
pub struct SlugAtlasLayout {
    /// Raw curves bytes, 24 bytes per curve, no padding.
    pub curves_data: Vec<u8>,
    /// Raw curve index bytes 4 bytes per u32.
    pub curve_indices_data: Vec<u8>,
    /// Raw glyph bytes 80 bytes per SlugGlyph struct.
    pub glyphs_data: Vec<u8>,
}

impl SlugAtlasLayout {
    /// Total bytes when packed sequentially.
    pub fn total_bytes(&self) -> usize {
        self.curves_data.len() + self.curve_indices_data.len() + self.glyphs_data.len()
    }

    /// Assert that the data fits within the for now 2048x2048 webgl texture
    /// limit chosen out of my ass to be roughly big enough for all JOYO kanji
    /// and ascii by my swag. I never tested it to find out how complex of
    /// curves they all had though. If not this is a fatal panic.
    pub fn assert_fits_webgl_textures(&self) {
        let max_texels = (SLUG_TEX_WIDTH * SLUG_TEX_WIDTH) as usize;

        // Curves are represented as 2 rgba32float texels per 24-byte curve.
        let curve_count = self.curves_data.len() / 24;
        let curve_texels = curve_count * 2;
        assert!(
            curve_texels <= max_texels,
            "SlugAtlasLayout: {} curves require ~{} texels but webgl texture \
             size is capped to {} ({}x{}). Reduce the visible glyph set or recompile with \
             SLUG_TEX_WIDTH. Or make this dynamic finally future mitch.",
            curve_count,
            curve_texels,
            max_texels,
            SLUG_TEX_WIDTH,
            SLUG_TEX_WIDTH
        );

        // Curve indices are 4 u32s per rgba32uint texel.
        let index_count = self.curve_indices_data.len() / 4;
        let index_texels = index_count.div_ceil(4);
        assert!(
            index_texels <= max_texels,
            "SlugAtlasLayout: {} curve indices require {} texels but webgl texture \
             size is {}. Reduce the visible glyph set or increase SLUG_TEX_WIDTH. \
             Or make this dynamic finally future mitch.",
            index_count,
            index_texels,
            max_texels
        );

        // Glyphs are 5 rgba32uint texels per 80-byte SlugGlyph.
        let glyph_count = self.glyphs_data.len() / 80;
        let glyph_texels = glyph_count * 5;
        assert!(
            glyph_texels <= max_texels,
            "SlugAtlasLayout: {} glyphs require {} texels but webgl texture \
             size is {}. Reduce the charset or increase SLUG_TEX_WIDTH. \
             Or make this dynamic finally future mitch.",
            glyph_count,
            glyph_texels,
            max_texels
        );
    }

    /// Build a `texture_2d<f32>` rgba32float Image for the bezier curves data.
    /// Each curve occupies 2 texels: (p0.x,p0.y,p1.x,p1.y) and (p2.x,p2.y,0,0).
    #[cfg(feature = "webgl")]
    pub fn curves_image(&self) -> bevy::prelude::Image {
        use bevy::asset::RenderAssetUsages;
        use bevy::prelude::Image;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

        let curve_count = self.curves_data.len() / 24;
        let texel_count = curve_count * 2;
        let width = SLUG_TEX_WIDTH;
        let height = ((texel_count as u32).div_ceil(width)).max(1);

        // Pad to full texel rows.
        let total_texels = (width * height) as usize;
        // 16 bytes per rgba32float texel
        let mut pixels = vec![0u8; total_texels * 16];

        for (ci, chunk) in self.curves_data.chunks_exact(24).enumerate() {
            let base = ci * 2 * 16;
            // texel 0 is p0.x p0.y p1.x p1.y
            pixels[base..base + 16].copy_from_slice(&chunk[0..16]);
            // texel 1 is p2.x p2.y 0 0
            pixels[base + 16..base + 24].copy_from_slice(&chunk[16..24]);
        }

        Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba32Float,
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    /// Build a `texture_2d<u32>` rgba32uint Image for the curve indices.
    /// 4 u32s are packed per texel.
    #[cfg(feature = "webgl")]
    pub fn curve_indices_image(&self) -> bevy::prelude::Image {
        use bevy::asset::RenderAssetUsages;
        use bevy::prelude::Image;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

        let index_count = self.curve_indices_data.len() / 4;
        let texel_count = index_count.div_ceil(4);
        let width = SLUG_TEX_WIDTH;
        let height = ((texel_count as u32).div_ceil(width)).max(1);

        let total_bytes = (width * height) as usize * 16;
        let mut pixels = vec![0u8; total_bytes];
        pixels[..self.curve_indices_data.len()].copy_from_slice(&self.curve_indices_data);

        Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba32Uint,
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    /// Build a `texture_2d<u32>` rgba32uint Image for the glyph data.
    /// Each SlugGlyph is 80 bytes = 5 x vec4<u32> that occupies 5 texels.
    #[cfg(feature = "webgl")]
    pub fn glyphs_image(&self) -> bevy::prelude::Image {
        use bevy::asset::RenderAssetUsages;
        use bevy::prelude::Image;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

        let glyph_count = self.glyphs_data.len() / 80;
        let texel_count = glyph_count * 5;
        let width = SLUG_TEX_WIDTH;
        let height = ((texel_count as u32).div_ceil(width)).max(1);

        let total_bytes = (width * height) as usize * 16;
        let mut pixels = vec![0u8; total_bytes];
        pixels[..self.glyphs_data.len()].copy_from_slice(&self.glyphs_data);

        Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba32Uint,
            RenderAssetUsages::RENDER_WORLD,
        )
    }
}

// AsBindGroup is implemented manually so we can supply sub-range BufferBindings
// pointing into different byte regions of the same underlying GPU buffer.
// The 0.18 API gives us a `&BindGroupLayoutDescriptor` + `&PipelineCache`;
// calling `pipeline_cache.get_bind_group_layout(layout)` yields the
// `BindGroupLayout` we pass to `render_device.create_bind_group(...)`.
/// Material for slug text rendering. Registered as both `UiMaterial` and
/// `Material2d` so the same atlas data can be used in UI nodes and 2D meshes.
///
/// Binding layout (both paths):
///   @binding(0)  uniform   SlugParams                16 bytes
///   @binding(1)  storage   curves[]                  native; `texture_2d<f32>` webgl
///   @binding(2)  storage   curve_indices[]           native; `texture_2d<u32>` webgl
///   @binding(3)  storage   glyphs[]                  native; `texture_2d<u32>` webgl
///
/// Native path: one packed ShaderStorageBuffer, three sub-range bindings.
/// `curves_offset/size`, `curve_indices_offset/size`, `glyphs_offset/size`
/// describe where each array lives inside the single buffer.
///
/// webgl path: three data textures (rgba32float / rgba32uint) accessed via
/// `textureLoad` with integer coordinates - no samplers needed.
#[cfg(not(feature = "webgl"))]
#[derive(Asset, TypePath, Clone)]
pub struct SlugMaterial {
    pub params: SlugParams,
    /// RGBA linear color applied at the call site in ui_text.wesl.
    /// Kept on the material because it is a per-draw-call concern, not a lib or
    /// atlas concern. The shader reads it as a push-constant-style field from
    /// the UiMaterial-local uniform instead of the slug lib uniform.
    pub text_color: Vec4,
    /// Pre-computed local-to-clip matrix for 3d Material path.
    pub local_to_clip: [[f32; 4]; 4],
    /// Single packed GPU buffer: [curves | curve_indices | glyphs] with alignment.
    pub atlas_buffer: Option<Handle<ShaderStorageBuffer>>,
    pub curves_offset: u64,
    pub curves_size: u64,
    pub curve_indices_offset: u64,
    pub curve_indices_size: u64,
    pub glyphs_offset: u64,
    pub glyphs_size: u64,
    /// SlugDrawData buffer: [runs | glyph_layout] with 256-byte alignment between.
    /// Binding 4 = runs sub-range of N x 16 bytes, one SlugRunDesc each.
    /// Binding 5 = glyph_layout sub-range of M x 48 bytes, one SlugGlyphLayout each.
    pub draw_buffer: Option<Handle<ShaderStorageBuffer>>,
    pub runs_offset: u64,
    pub runs_size: u64,
    pub glyph_layout_offset: u64,
    pub glyph_layout_size: u64,
}

#[cfg(not(feature = "webgl"))]
impl Default for SlugMaterial {
    fn default() -> Self {
        SlugMaterial {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            atlas_buffer: None,
            curves_offset: 0,
            curves_size: 0,
            curve_indices_offset: 0,
            curve_indices_size: 0,
            glyphs_offset: 0,
            glyphs_size: 0,
            draw_buffer: None,
            runs_offset: 0,
            runs_size: 0,
            glyph_layout_offset: 0,
            glyph_layout_size: 0,
        }
    }
}

#[cfg(not(feature = "webgl"))]
impl AsBindGroup for SlugMaterial {
    type Data = ();
    type Param = bevy::ecs::system::lifetimeless::SRes<
        bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderStorageBuffer>,
    >;

    fn label() -> &'static str {
        "slug_material"
    }

    fn bind_group_data(&self) -> Self::Data {}

    fn as_bind_group(
        &self,
        layout: &BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        gpu_buffers: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        // Retrieve the prepared GPU storage buffer.
        let atlas_gpu = match &self.atlas_buffer {
            Some(h) => gpu_buffers
                .get(h)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => return Err(AsBindGroupError::RetryNextUpdate),
        };

        let draw_gpu = match &self.draw_buffer {
            Some(h) => gpu_buffers
                .get(h)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => return Err(AsBindGroupError::RetryNextUpdate),
        };

        let mut params_bytes = [0u8; 96];
        params_bytes[..16].copy_from_slice(bytemuck::bytes_of(&self.params));
        params_bytes[16..32].copy_from_slice(bytemuck::bytes_of(&self.text_color));
        params_bytes[32..96].copy_from_slice(bytemuck::cast_slice(&self.local_to_clip));
        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("slug_params"),
            contents: &params_bytes,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &atlas_gpu.buffer,
                        offset: self.curves_offset,
                        size: BufferSize::new(self.curves_size),
                    }),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &atlas_gpu.buffer,
                        offset: self.curve_indices_offset,
                        size: BufferSize::new(self.curve_indices_size),
                    }),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &atlas_gpu.buffer,
                        offset: self.glyphs_offset,
                        size: BufferSize::new(self.glyphs_size),
                    }),
                },
                // binding 4 is a runs[] array of (SlugRunDesc array, 16 bytes each)
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &draw_gpu.buffer,
                        offset: self.runs_offset,
                        size: BufferSize::new(self.runs_size),
                    }),
                },
                // binding 5 is a glyph_layout[] array of (SlugGlyphLayout array, 48 bytes each)
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &draw_gpu.buffer,
                        offset: self.glyph_layout_offset,
                        size: BufferSize::new(self.glyph_layout_size),
                    }),
                },
            ],
        );

        Ok(PreparedBindGroup {
            bindings: BindingResources(vec![]),
            bind_group,
        })
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &bevy::render::renderer::RenderDevice,
        _param: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        Err(AsBindGroupError::CreateBindGroupDirectly)
    }

    fn bind_group_layout_entries(
        _: &bevy::render::renderer::RenderDevice,
        _: bool,
    ) -> Vec<BindGroupLayoutEntry>
    where
        Self: Sized,
    {
        // Helper closure for storage buffer layout entries.
        let storage_entry = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage_entry(1),
            storage_entry(2),
            storage_entry(3),
            storage_entry(4),
            storage_entry(5),
        ]
    }
}

// AsBindGroup is implemented manually for webgl because the derive macro does not have
// a `#[texture_no_sampler]` attribute and these textures cannot have samplers as
// textureLoad uses integer coordinates, not normalized UV sampling like native.

#[cfg(feature = "webgl")]
#[derive(Asset, TypePath, Clone)]
pub struct SlugMaterial {
    pub params: SlugParams,
    /// RGBA linear color applied at the call site. Not in SlugParams which is a lib
    /// concern. Kept here and packed into the uniform at binding 0 alongside
    /// node_size, same as the native path.
    pub text_color: Vec4,
    /// Pre-computed local-to-clip matrix for 3d path only. Unused for the webgl
    /// path but kept for struct parity.
    pub local_to_clip: [[f32; 4]; 4],
    /// curves_tex: texture_2d<f32>, rgba32float, 2 texels per curve.
    pub curves_image: Handle<Image>,
    /// curve_indices_tex: texture_2d<u32>, rgba32uint, 4 indices per texel.
    pub curve_indices_image: Handle<Image>,
    /// glyphs_tex: texture_2d<u32>, rgba32uint, 5 texels per glyph.
    pub glyphs_image: Handle<Image>,
    /// runs_tex: texture_2d<f32>, rgba32float, 1 texel per SlugRunDesc (16 bytes).
    pub runs_image: Handle<Image>,
    /// glyph_layout_tex: texture_2d<f32>, rgba32float, 3 texels per SlugGlyphLayout.
    pub glyph_layout_image: Handle<Image>,
}

#[cfg(feature = "webgl")]
impl Default for SlugMaterial {
    fn default() -> Self {
        SlugMaterial {
            params: SlugParams::default(),
            text_color: Vec4::ONE,
            local_to_clip: Mat4::IDENTITY.to_cols_array_2d(),
            curves_image: Handle::default(),
            curve_indices_image: Handle::default(),
            glyphs_image: Handle::default(),
            runs_image: Handle::default(),
            glyph_layout_image: Handle::default(),
        }
    }
}

fn slug_is_webgl() -> bool {
    cfg!(all(
        feature = "webgl",
        target_arch = "wasm32",
        not(feature = "webgpu")
    ))
}

fn slug_push_shader_defs(
    fragment: &mut bevy::render::render_resource::FragmentState,
    ui_material: bool,
) {
    fragment
        .shader_defs
        .push(ShaderDefVal::Bool("WEBGL".into(), slug_is_webgl()));
    fragment
        .shader_defs
        .push(ShaderDefVal::Bool("UI_MATERIAL".into(), ui_material));
}

// Binding layout:
//   @binding(0)  uniform   SlugParams
//   @binding(1)  texture   curves_tex        (texture_2d<f32>, rgba32float)
//   @binding(2)  texture   curve_indices_tex (texture_2d<u32>, rgba32uint)
//   @binding(3)  texture   glyphs_tex        (texture_2d<u32>, rgba32uint)

/// webgl AsBindGroup: three data textures accessed via textureLoad, no samplers.
/// Uses `as_bind_group` directly because texture views can't be owned in
/// `OwnedBindingResource::TextureView` sadly.
#[cfg(feature = "webgl")]
impl AsBindGroup for SlugMaterial {
    type Data = ();
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "slug_material_webgl"
    }

    fn bind_group_data(&self) -> Self::Data {}

    fn as_bind_group(
        &self,
        layout: &BindGroupLayoutDescriptor,
        render_device: &bevy::render::renderer::RenderDevice,
        pipeline_cache: &bevy::render::render_resource::PipelineCache,
        (images, _fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let get_view = |handle: &Handle<Image>| -> Result<
            &bevy::render::render_resource::TextureView,
            AsBindGroupError,
        > {
            // TODO: How do I deal with this without this hack? God I hate webgl
            // and all of its bullshit. I spend 300% more time keeping web stuff
            // working than linux/macos/windows combined. Worst. Platform. Ever.
            if handle.id() == bevy::asset::AssetId::default() {
                return Err(AsBindGroupError::RetryNextUpdate);
            }
            images
                .get(handle)
                .map(|gpu| &gpu.texture_view)
                .ok_or(AsBindGroupError::RetryNextUpdate)
        };

        let curves_view = get_view(&self.curves_image)?;
        let indices_view = get_view(&self.curve_indices_image)?;
        let glyphs_view = get_view(&self.glyphs_image)?;
        let runs_view = get_view(&self.runs_image)?;
        let layout_view = get_view(&self.glyph_layout_image)?;

        let mut params_bytes = [0u8; 96];
        params_bytes[..16].copy_from_slice(bytemuck::bytes_of(&self.params));
        params_bytes[16..32].copy_from_slice(bytemuck::bytes_of(&self.text_color));
        params_bytes[32..96].copy_from_slice(bytemuck::cast_slice(&self.local_to_clip));
        let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("slug_params_webgl"),
            contents: &params_bytes,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        let bg_layout = &pipeline_cache.get_bind_group_layout(layout);
        let bind_group = render_device.create_bind_group(
            Self::label(),
            bg_layout,
            &BindGroupEntries::sequential((
                params_buf.as_entire_binding(),
                curves_view,
                indices_view,
                glyphs_view,
                runs_view,
                layout_view,
            )),
        );

        Ok(PreparedBindGroup {
            bindings: BindingResources(vec![]),
            bind_group,
        })
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &bevy::render::renderer::RenderDevice,
        _param: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        Err(AsBindGroupError::CreateBindGroupDirectly)
    }

    fn bind_group_layout_entries(
        _: &bevy::render::renderer::RenderDevice,
        _: bool,
    ) -> Vec<BindGroupLayoutEntry>
    where
        Self: Sized,
    {
        let float_tex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uint_tex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Uint,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            float_tex(1),
            uint_tex(2),
            uint_tex(3),
            float_tex(4),
            float_tex(5),
        ]
    }
}

impl UiMaterial for SlugMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/slug/ui_text.wesl".into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        if let Some(ref mut fragment) = descriptor.fragment {
            slug_push_shader_defs(fragment, true);
        }
    }
}

impl Material2d for SlugMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/slug/text.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(ref mut fragment) = descriptor.fragment {
            slug_push_shader_defs(fragment, false);
        }
        Ok(())
    }
}

/// 3D Material impl for slug text meshes placed in world space.
///
/// Adds a third rendering surface alongside Material2d and UiMaterial.
/// The mesh geometry is normalized to 1 world unit tall; Transform handles
/// placement and scale. Uses the same atlas buffers and bind group layout as
/// the 2D path. Only the vertex shader differs
#[cfg(not(feature = "webgl"))]
impl Material for SlugMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/slug/mesh3d.wesl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "embedded://flan/slug/mesh3d.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor
            .vertex
            .shader_defs
            .push(ShaderDefVal::Bool("MATERIAL_3D".into(), true));
        if let Some(ref mut fragment) = descriptor.fragment {
            slug_push_shader_defs(fragment, false);
            fragment
                .shader_defs
                .push(ShaderDefVal::Bool("MATERIAL_3D".into(), true));
        }
        // Disable backface culling so the text is readable from both sides.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// 3D Material impl for webgl uses data textures at group 3
#[cfg(feature = "webgl")]
impl Material for SlugMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://flan/slug/mesh3d.wesl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "embedded://flan/slug/mesh3d.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor
            .vertex
            .shader_defs
            .push(ShaderDefVal::Bool("MATERIAL_3D".into(), true));
        if let Some(ref mut fragment) = descriptor.fragment {
            slug_push_shader_defs(fragment, false);
            fragment
                .shader_defs
                .push(ShaderDefVal::Bool("MATERIAL_3D".into(), true));
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Marker component for entities that are managed slug text nodes.
#[derive(Component)]
pub struct SlugText;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "webgl")]
    fn plot_points_uniform_webgl_alignment() {
        use std::mem::size_of;
        assert_eq!(
            size_of::<PlotPointsUniform>() % 16,
            0,
            "PlotPointsUniform must be a multiple of 16 bytes for webgl got {} bytes",
            size_of::<PlotPointsUniform>()
        );
    }

    #[test]
    fn slug_params_is_16_bytes() {
        assert_eq!(
            std::mem::size_of::<SlugParams>(),
            16,
            "SlugParams must be exactly 16 bytes (node_size vec2 + 8 bytes pad)"
        );
    }

    #[test]
    fn slug_run_desc_is_16_bytes() {
        assert_eq!(
            std::mem::size_of::<SlugRunDesc>(),
            16,
            "SlugRunDesc must be exactly 16 bytes (1 x vec4<f32>) for webgl texel packing"
        );
    }

    #[test]
    // TODO: LEAK [   0.229s] flan tests::max_plot_points_constant
    // I'm not sure this will survive a refactor into bevy rendering.
    fn max_plot_points_constant() {
        assert_eq!(MAX_PLOT_POINTS, 512);
    }
}
