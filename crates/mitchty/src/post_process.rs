// Custom post-processing system that dynamically loads shaders from flan/fullscreen/
use bevy::{
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::query::QueryItem,
    prelude::*,
    render::{
        RenderApp,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_graph::{
            NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::*,
        renderer::{RenderContext, RenderDevice},
        view::ViewTarget,
    },
    shader::ShaderDefVal,
};
use std::collections::HashMap;

/// Plugin that manages post-processing effects with dynamic shader loading
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
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<PostProcessPipeline>()
            .add_render_graph_node::<ViewNodeRunner<PostProcessNode>>(Core3d, PostProcessLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    PostProcessLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }
}

/// Render label for the post-processing node
#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct PostProcessLabel;

/// Component that configures post-processing settings for a camera
#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
#[require(Camera3d)]
pub struct PostProcessSettings {
    /// Effect intensity 0.0 passthrough 1.0 = most effect of effect
    pub intensity: f32,
    /// Time value for animated effects
    pub time: f32,
    // WebGL2 structs must be 16 byte aligned
    #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
    pub _webgl2_padding: Vec2,
}

/// Resource listing all available shader files
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
                name: "chromatic-aberration".to_string(),
                display_name: "Chromatic Aberration".to_string(),
            },
            ShaderInfo {
                name: "vhs-effect".to_string(),
                display_name: "VHS Effect".to_string(),
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

/// Resource tracking which shader is currently active
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

/// Resource to track whether effects should be enabled
#[derive(Resource, Default)]
pub struct EffectsEnabled(pub bool);

/// Render-world resource holding the post-processing pipeline.
#[derive(Resource)]
struct PostProcessPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipelines: HashMap<String, CachedRenderPipelineId>,
}

impl FromWorld for PostProcessPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        let layout = render_device.create_bind_group_layout(
            "post_process_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    BindGroupLayoutEntry {
                        binding: u32::MAX,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Texture {
                            sample_type: TextureSampleType::Float { filterable: true },
                            view_dimension: TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: u32::MAX,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: u32::MAX,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: Some(PostProcessSettings::min_size()),
                        },
                        count: None,
                    },
                ),
            ),
        );

        let sampler = render_device.create_sampler(&SamplerDescriptor::default());

        let mut pipelines = HashMap::new();

        let available_shaders = AvailableShaders::default();

        let webgl_active = cfg!(all(
            feature = "webgl",
            target_arch = "wasm32",
            not(feature = "webgpu")
        ));

        let mut shader_handles = Vec::new();
        {
            let asset_server = world.resource::<AssetServer>();
            let fullscreen_shader: Handle<Shader> = asset_server
                .load("embedded://bevy_core_pipeline/fullscreen_vertex_shader/fullscreen.wgsl");

            for shader_info in &available_shaders.shaders {
                let embedded_path = format!("embedded://flan/fullscreen/{}.wesl", shader_info.name);
                let shader_handle: Handle<Shader> = asset_server.load(embedded_path);
                shader_handles.push((
                    shader_info.name.clone(),
                    shader_handle,
                    fullscreen_shader.clone(),
                ));
            }
        }

        let pipeline_cache = world.resource_mut::<PipelineCache>();
        for (name, shader_handle, fullscreen_shader) in shader_handles {
            let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(format!("post_process_pipeline_{}", name).into()),
                layout: vec![BindGroupLayoutDescriptor {
                    label: "post_process_bind_group_layout_desc".into(),
                    entries: vec![
                        BindGroupLayoutEntry {
                            binding: 0,
                            visibility: ShaderStages::FRAGMENT,
                            ty: BindingType::Texture {
                                sample_type: TextureSampleType::Float { filterable: true },
                                view_dimension: TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        BindGroupLayoutEntry {
                            binding: 1,
                            visibility: ShaderStages::FRAGMENT,
                            ty: BindingType::Sampler(SamplerBindingType::Filtering),
                            count: None,
                        },
                        BindGroupLayoutEntry {
                            binding: 2,
                            visibility: ShaderStages::FRAGMENT,
                            ty: BindingType::Buffer {
                                ty: BufferBindingType::Uniform,
                                has_dynamic_offset: true,
                                min_binding_size: Some(PostProcessSettings::min_size()),
                            },
                            count: None,
                        },
                    ],
                }],
                vertex: VertexState {
                    shader: fullscreen_shader,
                    shader_defs: vec![],
                    entry_point: Some("fullscreen_vertex_shader".into()),
                    buffers: vec![],
                },
                fragment: Some(FragmentState {
                    shader: shader_handle,
                    shader_defs: vec![ShaderDefVal::Bool("WEBGL".into(), webgl_active)],
                    entry_point: Some("fragment".into()),
                    targets: vec![Some(ColorTargetState {
                        format: TextureFormat::bevy_default(),
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                push_constant_ranges: vec![],
                zero_initialize_workgroup_memory: false,
            });

            pipelines.insert(name.clone(), pipeline_id);
            debug!("queued pipeline for shader: {}", name);
        }

        Self {
            layout,
            sampler,
            pipelines,
        }
    }
}

/// The post-processing render node
#[derive(Default)]
struct PostProcessNode;

impl ViewNode for PostProcessNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static DynamicUniformIndex<PostProcessSettings>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, settings_index): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<PostProcessPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let available_shaders = world.resource::<AvailableShaders>();
        let active_shader = world.resource::<ActiveShader>();

        let active_shader_name = &available_shaders.shaders[active_shader.index].name;

        let Some(pipeline_id) = pipeline.pipelines.get(active_shader_name) else {
            return Ok(());
        };

        let Some(render_pipeline) = pipeline_cache.get_render_pipeline(*pipeline_id) else {
            return Ok(());
        };

        let settings_uniforms = world.resource::<ComponentUniforms<PostProcessSettings>>();
        let Some(settings_binding) = settings_uniforms.uniforms().binding() else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();

        let bind_group = render_context.render_device().create_bind_group(
            "post_process_bind_group",
            &pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &pipeline.sampler,
                settings_binding.clone(),
            )),
        );

        let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("post_process_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                resolve_target: None,
                ops: Operations::default(),
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_render_pipeline(render_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[settings_index.index()]);
        render_pass.draw(0..3, 0..1);

        Ok(())
    }
}
