// Camera setup assumptions for headless rendering.
//
// Mostly abused this to help learn what is/n't present now in 0.19 and how
// metal only was failing for whatever reason. Not sure they are useful tests
// tbh but keepin em until future me gets angy.
#[cfg(test)]
pub mod tests {
    use bevy::asset::Handle;
    use bevy::camera::{Camera, CameraOutputMode, ClearColor, ClearColorConfig};
    use bevy::camera::{NormalizedRenderTarget, RenderTarget};
    use bevy::image::Image;
    use bevy::math::UVec2;
    use bevy::prelude::Assets;
    use bevy::render::render_resource::TextureFormat;
    use bevy::window::WindowRef;

    // 0.19 world clear color bs
    #[test]
    fn world_default_clear_color_is_clear_color_opaque() {
        use bevy::color::Srgba;
        let clear = ClearColor::default();
        let Srgba {
            red,
            green,
            blue,
            alpha,
        } = Srgba::from(clear.0);
        let r = (red * 255.0).round() as u8;
        let g = (green * 255.0).round() as u8;
        let b = (blue * 255.0).round() as u8;
        let a = (alpha * 255.0).round() as u8;
        assert_eq!(
            [r, g, b, a],
            [43, 44, 47, 255],
            "ClearColor::default() must encode to [43,44,47,255] sRGB bytes;\
             this is exactly what the canary sees when the UiMaterial pass is skipped"
        );
    }

    // Camera clear color
    #[test]
    fn camera_default_clear_color_field_uses_world_resource() {
        assert!(
            matches!(Camera::default().clear_color, ClearColorConfig::Default),
            "Camera::default().clear_color must be ClearColorConfig::Default\
             (uses world ClearColor resource)"
        );
    }

    // The camera output write clear color is as expected for metal
    #[test]
    fn camera_output_mode_write_clear_color_also_defaults_to_world_resource() {
        match Camera::default().output_mode {
            CameraOutputMode::Write { clear_color, .. } => assert!(
                matches!(clear_color, ClearColorConfig::Default),
                "CameraOutputMode::Write.clear_color must default to ClearColorConfig::Default as setting Camera.clear_color alone cannot prevent the output write from clearing the world ClearColor"
            ),
            other => panic!(
                "Camera::default().output_mode must be CameraOutputMode::Write, got {other:?}"
            ),
        }
    }

    #[test]
    fn image_render_target_normalizes_without_primary_window() {
        let handle = Handle::<Image>::default();
        let rt = RenderTarget::from(handle);
        let normalized = rt.normalize(None);
        assert!(
            normalized.is_some(),
            "RenderTarget::Image must normalize to Some even with no primary window entity. If normalize() returns None the camera_system skips the camera and computed.target_info is never set"
        );
        assert!(
            matches!(normalized.unwrap(), NormalizedRenderTarget::Image(_)),
            "normalized render target must be the Image variant"
        );
    }

    #[test]
    fn window_primary_render_target_returns_none_without_primary_window() {
        let rt = RenderTarget::Window(WindowRef::Primary);
        assert!(
            rt.normalize(None).is_none(),
            "RenderTarget::Window(Primary) must return None when no primary window exist as this is why all headless test cameras must use RenderTarget::Image to function here"
        );
    }
    #[test]
    fn camera_physical_target_size_is_none_before_camera_system_runs() {
        assert!(
            Camera::default().physical_target_size().is_none(),
            "Camera::default().physical_target_size() must be None as computed.target_info is only set by camera_system in PostUpdate. If camera_system errors out due to say an image not in Assets, this stays None and UiCameraView is never set up."
        );
    }
    #[test]
    fn make_render_target_creates_256x256_rgba8_unorm_srgb_image() {
        use crate::test::lib::bevy::RENDER_SIZE;
        // Assets::<Image>::default() is an in-memory store.
        let mut images = Assets::<Image>::default();
        let handle = crate::test::lib::bevy::make_render_target(&mut images);
        let image = images
            .get(&handle)
            .expect("render target image must be in Assets immediately after make_render_target");
        assert_eq!(
            image.size(),
            UVec2::new(RENDER_SIZE, RENDER_SIZE),
            "render target must be {RENDER_SIZE}x{RENDER_SIZE}; \
             a (0,0)-sized image would cause camera_system to return \
             physical_size=(0,0) and the UI guard to skip the camera"
        );
        assert_eq!(
            image.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb,
            "render target must be Rgba8UnormSrgb so sRGB(0,0,0) = byte 0 and \
             the canary solid-black assertion ([0,0,0,255]) is correct"
        );
    }

    #[test]
    fn canary_pixel_buffer_is_render_size_squared_times_four_bytes() {
        use crate::test::lib::bevy::RENDER_SIZE;
        let n = RENDER_SIZE as usize;
        let expected_bytes = n * n * 4;
        assert_eq!(
            expected_bytes,
            256 * 256 * 4,
            "RENDER_SIZE must be 256; buffer = RENDER_SIZE^2 x 4 RGBA bytes"
        );
        // 65536 pixels x 4 channels = 262144 bytes for our test buffers.
        assert_eq!(expected_bytes, 262144);
    }

    #[test]
    fn canary_solid_black_vec4_encodes_to_black_in_rgba8_unorm_srgb() {
        // Note alpha channels not encoded
        let linear_black = [0.0f32, 0.0f32, 0.0f32, 1.0f32];
        let expected_bytes: [u8; 4] = linear_black.map(|c| (c * 255.0).round() as u8);
        assert_eq!(
            expected_bytes,
            [0, 0, 0, 255],
            "vec4(0,0,0,1) must encode to [0,0,0,255] in Rgba8UnormSrgb"
        );

        let world_clear_bytes: [u8; 4] = [43, 44, 47, 255];
        assert_ne!(
            world_clear_bytes,
            [0, 0, 0, 255],
            "world clear color cannot be the canary solid-black clear"
        );
    }
}
