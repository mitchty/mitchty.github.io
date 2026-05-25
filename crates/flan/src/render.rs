// Abused to render to a wgpu device for unit tests right now. If I ever find a
// need to use this outside of tests its here for the taking.
//
// Note this has no window or event loop, its a oneshot stupid renderer.

use wgpu::util::DeviceExt;

// On Linux headless, concurrent wgpu device instances compete for the same
// driver resources and deadlock. Serialize all render_shader calls globally.
static RENDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Output size, 256x256 makes small png files and is faster to test.
pub const RENDER_SIZE: u32 = 256;

// RGBA pixel data, row major, top to bottom order
pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// This is a wgsl binding used as the first uniforms data
pub struct Binding<'a> {
    /// slot is 0 by default, here in case I need to do others
    pub slot: u32,
    /// Is this a StorageBuffer (new hotness) or a Uniform (old n busted)
    pub kind: BindingKind,
    /// Raw bytes to yeet at the shader
    pub data: &'a [u8],
}

/// Storage buffer or a Uniform for input data
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Uniform,
    StorageRead,
}

/// Blocking render function:
/// - compiles the wgsl shader
/// - uploads any bindings to first uniform group
/// - draws one full-size RENDER_SIZE x RENDER_SIZE frame image
///
/// Uses @group(0) for tests mostly.
///
/// Iff wgpu fails returns an error string from the Error from wgpu.
pub fn render_shader(wgsl_source: &str, bindings: &[Binding<'_>]) -> Result<RenderedFrame, String> {
    let _guard = RENDER_LOCK.lock().unwrap();
    pollster::block_on(render_async(
        RENDER_SIZE,
        RENDER_SIZE,
        wgsl_source,
        bindings,
    ))
}

/// Same as `render_shader` but renders to an explicit `width x height` instead
/// of the default square `RENDER_SIZE x RENDER_SIZE`. Useful for testing
/// rectangular viewports e.g. wide UI labels or anything that might need non
/// square output.
pub fn render_shader_sized(
    width: u32,
    height: u32,
    wgsl_source: &str,
    bindings: &[Binding<'_>],
) -> Result<RenderedFrame, String> {
    let _guard = RENDER_LOCK.lock().unwrap();
    pollster::block_on(render_async(width, height, wgsl_source, bindings))
}

async fn render_async(
    w: u32,
    h: u32,
    wgsl_source: &str,
    bindings: &[Binding<'_>],
) -> Result<RenderedFrame, String> {
    // Try native renderers first.
    //
    // I might consider making the backend to be choosable so I can try software
    // first for a reference render and have that compare across any of the
    // other backends.
    //
    // Software is meant more for ci, but not sure how to handle rendering to
    // specialized stuff without actual hardware SOMEWHERE.
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN
            | wgpu::Backends::METAL
            | wgpu::Backends::DX12
            | wgpu::Backends::GL,
        ..Default::default()
    });

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .map_err(|e| format!("no wgpu adapter found: {e}"))?;

    // Query what the adapter actually supports so we can ask for as many
    // storage buffers per stage as the hardware will give us. This is needed
    // for the Slug text renderer which uses 5 storage bindings in the fragment
    // stage. `downlevel_defaults()` only guarantees 4, so we start from the
    // adapter's reported limits and cap at that ceiling.
    //
    // TODO: This is why I need to not do array of structs per binding and
    // instead a single array of structs for everything in one binding. This
    // craps fiddly af.
    let adapter_limits = adapter.limits();
    let required_limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: adapter_limits
            .max_storage_buffers_per_shader_stage
            .max(8),
        ..wgpu::Limits::downlevel_defaults()
    };

    let (device, queue): (wgpu::Device, wgpu::Queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("shader-test-device"),
            required_features: wgpu::Features::empty(),
            required_limits,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("device request failed: {e}"))?;

    let texture: wgpu::Texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render-target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Readback buffer for the rendered image data. Note alignment needs to fit
    // the render size for the bytes per row. Future me can make this dynamic.
    let bytes_per_pixel: u32 = 4;
    let unpadded_row = w * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_row = unpadded_row.div_ceil(align) * align;

    let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // Setup the gpu buffers for the binding data, hacky af to work around the
    // borrow checker being angy about `gpu_bufs.push` mut getting indexed from
    // `as_entire_binding`.
    let gpu_bufs: Vec<wgpu::Buffer> = bindings
        .iter()
        .map(|b| {
            let usage = match b.kind {
                BindingKind::Uniform => wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                BindingKind::StorageRead => {
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST
                }
            };
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("binding-{}", b.slot)),
                contents: b.data,
                usage,
            })
        })
        .collect();

    let bgl_entries: Vec<wgpu::BindGroupLayoutEntry> = bindings
        .iter()
        .map(|b| wgpu::BindGroupLayoutEntry {
            binding: b.slot,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: match b.kind {
                    BindingKind::Uniform => wgpu::BufferBindingType::Uniform,
                    BindingKind::StorageRead => {
                        wgpu::BufferBindingType::Storage { read_only: true }
                    }
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();

    let bg_entries: Vec<wgpu::BindGroupEntry<'_>> = bindings
        .iter()
        .zip(gpu_bufs.iter())
        .map(|(b, buf)| wgpu::BindGroupEntry {
            binding: b.slot,
            resource: buf.as_entire_binding(),
        })
        .collect();

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bgl"),
        entries: &bgl_entries,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bg"),
        layout: &bgl,
        entries: &bg_entries,
    });

    // Crappy vertex shader that produces a full screen triangle uv.
    //
    // Fragment shaders need an entry point named `fragment` and write out RGBA
    // data as a `vec4<f32` to @location(0)
    let vs_src = r#"
        struct VsOut {
            @builtin(position) pos: vec4<f32>,
            @location(0)       uv:  vec2<f32>,
        }
        @vertex
        fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
            let x = f32((vi & 1u) << 2u) - 1.0;
            let y = f32((vi & 2u) << 1u) - 1.0;
            var out: VsOut;
            out.pos = vec4<f32>(x, y, 0.0, 1.0);
            out.uv  = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
            return out;
        }
    "#;

    let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("vs"),
        source: wgpu::ShaderSource::Wgsl(vs_src.into()),
    });
    let fs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fs"),
        source: wgpu::ShaderSource::Wgsl(wgsl_source.into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("layout"),
        bind_group_layouts: &[&bgl],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some("fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    // Actually render this crap
    let mut enc =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });

    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                depth_slice: None, // required in wgpu 27 for non-3D textures
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_pipeline(&pipeline);
        rp.set_bind_group(0, &bind_group, &[]);
        rp.draw(0..3, 0..1);
    }

    // Yeet the rendered image back
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback_buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(std::iter::once(enc.finish()));

    let buf_slice = readback_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buf_slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).unwrap();
    });

    // Actively pump the device until the map callback fires. A single
    // blocking poll() hangs on Linux headless/CI (Vulkan/GL need the device
    // driven in a loop; Metal on macOS drives itself). Poll::wait_for_map_async
    // is not yet stable across backends so we spin with a short timeout instead.
    loop {
        device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| format!("poll failed: {e}"))?;

        match rx.try_recv() {
            Ok(result) => {
                result.map_err(|e| format!("buffer map failed: {e}"))?;
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("map_async sender dropped unexpectedly".into());
            }
        }
    }

    let mapped = buf_slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * bytes_per_pixel) as usize);
    for row in 0..h {
        let start = (row * padded_row) as usize;
        let end = start + unpadded_row as usize;
        pixels.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    readback_buf.unmap();

    Ok(RenderedFrame {
        width: w,
        height: h,
        pixels,
    })
}
