// Canary material and headless render function.
#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::render_resource::*;
    use bevy::shader::ShaderRef;
    use bevy::ui::UiTargetCamera;
    use std::sync::{Arc, Mutex};

    use crate::shaders::{ShadersPlugin, canary_fill_shader_handle};
    use crate::test::lib::GPU_RENDER_LOCK;
    use crate::test::lib::bevy::{
        CaptureShared, CaptureState, HeadlessCapturePlugin, RENDER_SIZE, RenderedFrame,
        make_render_target, run_and_capture,
    };

    #[derive(Asset, AsBindGroup, TypePath, Clone)]
    pub struct CanaryMaterial {}

    impl UiMaterial for CanaryMaterial {
        fn fragment_shader() -> ShaderRef {
            ShaderRef::Handle(canary_fill_shader_handle())
        }

        fn specialize(_descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {}
    }

    fn build_headless_canary_app() -> App {
        let mut app = crate::test::lib::bevy::build_headless_app_base();
        app.add_plugins(ShadersPlugin);
        app.add_plugins(UiMaterialPlugin::<CanaryMaterial>::default());
        app
    }

    /// Render a solid-black 256x256 frame with `CanaryMaterial` backing it.
    ///
    /// Returns `None` when no GPU adapter is available aka CI without GPU (need to validate this).
    /// Otherwise returns the RGBA pixel data for the render.
    pub fn render_canary() -> Option<RenderedFrame> {
        let _guard = GPU_RENDER_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let state: CaptureShared = Arc::new(Mutex::new(CaptureState::Pending));
        let mut app = build_headless_canary_app();

        let image_handle = make_render_target(&mut app.world_mut().resource_mut::<Assets<Image>>());

        app.add_plugins(HeadlessCapturePlugin {
            handle: image_handle.clone(),
            state: state.clone(),
            min_frames: 0,
        });

        let ih = image_handle.clone();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut materials: ResMut<Assets<CanaryMaterial>>| {
                let mat = materials.add(CanaryMaterial {});

                let cam = commands
                    .spawn((
                        Camera2d,
                        crate::test::lib::bevy::headless_camera(),
                        RenderTarget::from(ih.clone()),
                    ))
                    .id();

                commands
                    .spawn((
                        Node {
                            width: Val::Px(RENDER_SIZE as f32),
                            height: Val::Px(RENDER_SIZE as f32),
                            ..default()
                        },
                        UiTargetCamera(cam),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            MaterialNode(mat),
                            Node {
                                width: Val::Px(RENDER_SIZE as f32),
                                height: Val::Px(RENDER_SIZE as f32),
                                ..default()
                            },
                        ));
                    });
            },
        );

        run_and_capture(&mut app, &state)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use inner::{CanaryMaterial, render_canary};
