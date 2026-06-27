// Plugin for PostProcess full screen shaders. Figured its better to have this
// in flan and not mitchty.
use bevy::{
    core_pipeline::{Core3d, Core3dSystems, FullscreenShader, tonemapping::tonemapping},
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        globals::GlobalsBuffer,
        render_resource::*,
        renderer::{RenderContext, RenderDevice},
        view::{ExtractedView, ViewTarget},
    },
    shader::ShaderDefVal,
};
use std::collections::HashMap;

/// Plugin that manages post-processing effects with dynamic shader loading.
pub struct PostProcessPlugin;

impl Plugin for PostProcessPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<PostProcessSettings>::default(),
            UniformComponentPlugin::<PostProcessSettings>::default(),
            ExtractResourcePlugin::<AvailableShaders>::default(),
            ExtractResourcePlugin::<ActiveShader>::default(),
        ))
        .init_resource::<AvailableShaders>()
        .init_resource::<ActiveShader>()
        .init_resource::<EffectsEnabled>();

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_systems(RenderStartup, |world: &mut World| {
                init_post_process_pipeline(world)
            })
            .add_systems(
                Core3d,
                post_process_system
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}

#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
#[require(Camera3d)]
pub struct PostProcessSettings {
    // TODO: Needs a refactor, intensity will change in future right now I mostly
    // wanted to fix bugs I found whilst migrating to 0.19 first.
    /// Effect intensity: 0.0 = passthrough, 1.0 = full effect.
    pub intensity: f32,
    // webgl 16-byte alignment padding for webgl bs
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub _pad0: f32,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub _pad1: f32,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub _pad2: f32,
}

#[derive(Resource, Clone, ExtractResource)]
pub struct AvailableShaders {
    pub shaders: Vec<ShaderInfo>,
}

#[derive(Clone)]
pub struct ShaderInfo {
    pub name: String,
    pub display_name: String,
}

impl Default for AvailableShaders {
    fn default() -> Self {
        let shaders = vec![
            ShaderInfo {
                name: "em-interference".to_string(),
                display_name: "EM Interference".to_string(),
            },
            ShaderInfo {
                name: "vhs-effect".to_string(),
                display_name: "VHS Effect".to_string(),
            },
            ShaderInfo {
                name: "chromatic-aberration".to_string(),
                display_name: "Chromatic Aberration".to_string(),
            },
            ShaderInfo {
                name: "oil-painting".to_string(),
                display_name: "Oil Painting".to_string(),
            },
            ShaderInfo {
                name: "edge-cartoon".to_string(),
                display_name: "Edge Cartoon".to_string(),
            },
        ];

        Self { shaders }
    }
}

/// Resource for the active/loaded shader in use index for the available shader Resource.
//TODO: Refactor this into a struct this is getting silly the two Resources.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct ActiveShader {
    pub index: usize,
}

impl ActiveShader {
    pub fn display_name<'a>(&self, available: &'a AvailableShaders) -> &'a str {
        &available.shaders[self.index].display_name
    }

    pub fn next(&mut self, available: &AvailableShaders) {
        self.index = (self.index + 1) % available.shaders.len();
    }

    pub fn previous(&mut self, available: &AvailableShaders) {
        if self.index == 0 {
            self.index = available.shaders.len() - 1;
        } else {
            self.index -= 1;
        }
    }
}

/// Resource to track whether effects should be enabled in the post process
/// pipeline or not.
#[derive(Resource, Default)]
pub struct EffectsEnabled(pub bool);

/// Render-world resource holding all of the post-processing pipelines.
#[derive(Resource)]
struct PostProcessPipeline {
    /// group(0): screen texture + sampler + settings uniform.
    ///
    /// Stored as a descriptor so we can call `pipeline_cache.get_bind_group_layout()`
    /// at bind-group-creation time. That method returns the exact same
    /// `BindGroupLayout` object the pipeline cache baked into the compiled
    /// pipeline, so wgpu accepts the bind group without complaint.
    layout0_desc: BindGroupLayoutDescriptor,

    /// group(1): globals uniform (time, delta_time, frame_count etc).
    layout1_desc: BindGroupLayoutDescriptor,

    sampler: Sampler,

    /// One compiled pipeline id per (shader_name, TextureFormat).
    pipelines: HashMap<(String, TextureFormat), CachedRenderPipelineId>,

    /// Effect and vertex shader handles, kept alive so assets aren't dropped by
    /// bevy gc.
    shader_handles: HashMap<String, (Handle<Shader>, Handle<Shader>)>,
}

/// Per-view component where one bind group exists per texture so that the draw
/// system can pick the right bind group without creating a new bind group every
/// frame and leak memory again. I must be using bind groups or materials wrong
/// in bevy have to look more in depth over weekend.
fn build_pipeline_descriptor(
    name: &str,
    format: TextureFormat,
    layout0_desc: &BindGroupLayoutDescriptor,
    layout1_desc: &BindGroupLayoutDescriptor,
    effect_handle: &Handle<Shader>,
    fullscreen_handle: &Handle<Shader>,
    webgl_active: bool,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(format!("post_process_pipeline_{name}_{format:?}").into()),
        layout: vec![layout0_desc.clone(), layout1_desc.clone()],
        vertex: VertexState {
            shader: fullscreen_handle.clone(),
            shader_defs: vec![],
            entry_point: Some("fullscreen_vertex_shader".into()),
            buffers: vec![],
        },
        fragment: Some(FragmentState {
            shader: effect_handle.clone(),
            shader_defs: vec![ShaderDefVal::Bool("WEBGL".into(), webgl_active)],
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        immediate_size: 0,
        zero_initialize_workgroup_memory: false,
    }
}

fn init_post_process_pipeline(world: &mut World) {
    let render_device = world.resource::<RenderDevice>();

    // group(0): screen texture, sampler, per-camera settings.
    let layout0_desc = BindGroupLayoutDescriptor::new(
        "post_process_layout0",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                binding_types::texture_2d(TextureSampleType::Float { filterable: true }),
                binding_types::sampler(SamplerBindingType::Filtering),
                binding_types::uniform_buffer::<PostProcessSettings>(true),
            ),
        ),
    );

    // group(1): globals uniform (time etc.) - matches bevy's GlobalsUniform layout.
    let layout1_desc = BindGroupLayoutDescriptor::new(
        "post_process_layout1",
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: Some(bevy::render::globals::GlobalsUniform::min_size()),
            },
            count: None,
        }],
    );

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let webgl_active = cfg!(all(
        feature = "webgl",
        target_arch = "wasm32",
        not(feature = "webgpu")
    ));

    let available_shaders = AvailableShaders::default();
    let mut shader_handles: HashMap<String, (Handle<Shader>, Handle<Shader>)> = HashMap::new();
    {
        let fullscreen_shader = world.resource::<FullscreenShader>();
        let fullscreen_handle = fullscreen_shader.shader();

        for shader_info in &available_shaders.shaders {
            let effect_handle: Handle<Shader> = match shader_info.name.as_str() {
                "chromatic-aberration" => crate::shaders::chromatic_aberration_shader_handle(),
                "vhs-effect" => crate::shaders::vhs_effect_shader_handle(),
                "em-interference" => crate::shaders::em_interference_shader_handle(),
                "oil-painting" => crate::shaders::oil_painting_shader_handle(),
                "edge-cartoon" => crate::shaders::edge_cartoon_shader_handle(),
                "cartoon-filter" => crate::shaders::cartoon_filter_shader_handle(),
                other => panic!("unknown fullscreen shader: {other}"),
            };
            shader_handles.insert(
                shader_info.name.clone(),
                (effect_handle, fullscreen_handle.clone()),
            );
        }
    }

    // Pre-queue pipelines for the two most common LDR formats. Any other
    // formats are queued later lazily in the prepare system when we first see a
    // view with that format.
    let common_formats = [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb];
    let mut pipelines = HashMap::new();
    {
        let pipeline_cache = world.resource::<PipelineCache>();
        for format in common_formats {
            for (name, (effect_handle, fullscreen_handle)) in &shader_handles {
                let desc = build_pipeline_descriptor(
                    name,
                    format,
                    &layout0_desc,
                    &layout1_desc,
                    effect_handle,
                    fullscreen_handle,
                    webgl_active,
                );
                let id = pipeline_cache.queue_render_pipeline(desc);
                pipelines.insert((name.clone(), format), id);
                debug!("queued pipeline: {name} format={format:?}");
            }
        }
    }

    world.insert_resource(PostProcessPipeline {
        layout0_desc,
        layout1_desc,
        sampler,
        pipelines,
        shader_handles,
    });
}

/// Post-process draw system.
///
/// Runs in `Core3dSystems::PostProcess`. Build all bind groups inline each
/// frame to avoid the `ViewQuery::single().unwrap()` panic footgun that fires
/// whenever any required component is temporarily absent (e.g.
/// `DynamicUniformIndex` not yet prepared on frame 0).
#[allow(clippy::too_many_arguments)]
fn post_process_system(
    views: Query<(
        &ViewTarget,
        &DynamicUniformIndex<PostProcessSettings>,
        &PostProcessSettings,
        &ExtractedView,
    )>,
    pipeline_res: Option<Res<PostProcessPipeline>>,
    pipeline_cache: Option<Res<PipelineCache>>,
    available_shaders: Res<AvailableShaders>,
    active_shader: Res<ActiveShader>,
    settings_uniforms: Option<Res<ComponentUniforms<PostProcessSettings>>>,
    globals_buffer: Option<Res<GlobalsBuffer>>,
    render_device: Option<Res<RenderDevice>>,
    mut ctx: RenderContext,
) {
    let Some(pipeline) = pipeline_res else {
        return;
    };
    let Some(pipeline_cache) = pipeline_cache else {
        return;
    };
    let Some(render_device) = render_device else {
        return;
    };

    let active_name = &available_shaders.shaders[active_shader.index].name;

    // Bail early if shared resources aren't yet ready/compiled/bound.
    let Some(settings_binding) = settings_uniforms
        .as_ref()
        .and_then(|u| u.uniforms().binding())
    else {
        return;
    };
    let Some(globals_binding) = globals_buffer.as_ref().and_then(|g| g.buffer.binding()) else {
        return;
    };

    for (view_target, settings_index, settings, extracted_view) in &views {
        if settings.intensity == 0.0 {
            continue;
        }

        let target_format = extracted_view.target_format;

        // Lazily queue a pipeline for target formats not covered at init time.
        if !pipeline
            .pipelines
            .contains_key(&(active_name.clone(), target_format))
        {
            if let Some((effect_handle, fullscreen_handle)) =
                pipeline.shader_handles.get(active_name)
            {
                let webgl_active = cfg!(all(
                    feature = "webgl",
                    target_arch = "wasm32",
                    not(feature = "webgpu")
                ));
                let desc = build_pipeline_descriptor(
                    active_name,
                    target_format,
                    &pipeline.layout0_desc,
                    &pipeline.layout1_desc,
                    effect_handle,
                    fullscreen_handle,
                    webgl_active,
                );
                pipeline_cache.queue_render_pipeline(desc);
            }
            // Pipeline is now queued but not yet compiled so skip this frame/tick.
            continue;
        }

        let Some(pipeline_id) = pipeline
            .pipelines
            .get(&(active_name.clone(), target_format))
        else {
            continue;
        };

        let Some(render_pipeline) = pipeline_cache.get_render_pipeline(*pipeline_id) else {
            continue;
        };

        let layout0 = pipeline_cache.get_bind_group_layout(&pipeline.layout0_desc);
        let layout1 = pipeline_cache.get_bind_group_layout(&pipeline.layout1_desc);

        let post_process = view_target.post_process_write();
        let source = post_process.source;
        let destination = post_process.destination;

        let bind_group_0 = render_device.create_bind_group(
            "post_process_bind_group",
            &layout0,
            &BindGroupEntries::sequential((source, &pipeline.sampler, settings_binding.clone())),
        );

        let bind_group_1 = render_device.create_bind_group(
            "post_process_globals_bind_group",
            &layout1,
            &BindGroupEntries::single(globals_binding.clone()),
        );

        let mut render_pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("post_process_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: destination,
                    resolve_target: None,
                    ops: Operations::default(),
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

        render_pass.set_pipeline(render_pipeline);
        render_pass.set_bind_group(0, &bind_group_0, &[settings_index.index()]);
        render_pass.set_bind_group(1, &bind_group_1, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
