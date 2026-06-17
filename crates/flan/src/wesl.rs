// Shader variant flags used by the test harness to select the headless render
// path (wgpu_test) and to gate WebGL / UI-material code paths.
// TODO: There's enough changed already I'll rip this other human tail off
// later. wesl is gone entirely now.

/// Shader variant descriptor used by test render helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    pub ui_material: bool,
    pub webgl: bool,
    /// Set WGPU_TEST=true
    pub wgpu_test: bool,
}

impl Variant {
    pub const MATERIAL: Self = Self {
        ui_material: false,
        webgl: false,
        wgpu_test: false,
    };
    pub const UI: Self = Self {
        ui_material: true,
        webgl: false,
        wgpu_test: false,
    };
    pub const WEBGL: Self = Self {
        ui_material: false,
        webgl: true,
        wgpu_test: false,
    };
    pub const WEBGL_UI: Self = Self {
        ui_material: true,
        webgl: true,
        wgpu_test: false,
    };

    pub const TEST_MATERIAL: Self = Self {
        ui_material: false,
        webgl: false,
        wgpu_test: true,
    };
    pub const TEST_UI: Self = Self {
        ui_material: true,
        webgl: false,
        wgpu_test: true,
    };
    pub const TEST_WEBGL: Self = Self {
        ui_material: false,
        webgl: true,
        wgpu_test: true,
    };
    pub const TEST_WEBGL_UI: Self = Self {
        ui_material: true,
        webgl: true,
        wgpu_test: true,
    };

    pub fn dir_name(self) -> &'static str {
        match (self.ui_material, self.webgl) {
            (false, false) => "material",
            (true, false) => "ui",
            (false, true) => "webgl",
            (true, true) => "webgl-ui",
        }
    }
}
