// Unit testing for wesl in wgpu, only meant for backend testing shader output
// via ssim.
//
// Its kinda jank af but it sorta works well enough.
use wesl::syntax::ModulePath;

const PLOT_TYPES_WESL: &str = include_str!("lib/types/plot.wesl");
const PLOT_BINDINGS_WESL: &str = include_str!("lib/bindings/plot.wesl");
const PLOT_INPUT_WESL: &str = include_str!("lib/input/plot.wesl");
const PLOT_HELPERS_WESL: &str = include_str!("lib/helpers/plot.wesl");

const FULLSCREEN_EFFECT_TYPES_WESL: &str = include_str!("lib/types/fullscreen_effect.wesl");
const FULLSCREEN_EFFECT_BINDINGS_WESL: &str = include_str!("lib/bindings/fullscreen_effect.wesl");
const FULLSCREEN_EFFECT_INPUT_WESL: &str = include_str!("lib/input/fullscreen_effect.wesl");

/// Shader variant used in wgpu to separate out webgl shaders and not.
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

/// Convert a path string from a shader to a `ModulePath`
///
/// Leading "/" -> PathOrigin::Absolute, mimics Bevy from_wesl
/// No leading "/" -> PathOrigin::Package(first_segment) kinda relative import
fn mp(path: &str) -> ModulePath {
    ModulePath::from_path(std::path::Path::new(path))
}

/// Build a `VirtualResolver` with all lib wesl helpers registered under their
/// absolute Bevy asset paths so things seem sane from a wesl shader pov.
///
/// The keys use `from_path("/flan/shaders/lib/...")` which yields:
///   PathOrigin::Absolute, components: ["flan", "shaders", "lib", ...]
///
/// This matches what the WESL grammar produces for:
///   import package::flan::shaders::lib::...::{...}
fn make_resolver<'a>(top_level_stem: &str, top_level_src: &'a str) -> wesl::VirtualResolver<'a> {
    let mut r = wesl::VirtualResolver::new();

    for (path_str, src) in HELPERS {
        r.add_module(mp(path_str), (*src).to_owned().into());
    }

    // Register the top-level shader
    // e.g. "fullscreen/chromatic-aberration" -> "/flan/shaders/fullscreen/chromatic-aberration"
    let top_path = format!("/flan/shaders/{top_level_stem}");
    r.add_module(mp(&top_path), top_level_src.to_owned().into());

    r
}

/// Lib helpers to make the unit tests work close enough to how the bevy wesl stuff does.
/// TODO: ALL THIS JUNK NEEDS TO BE DYNAMIC at some point. Future mitch problem.
const HELPERS: &[(&str, &str)] = &[
    ("/flan/shaders/lib/types/plot", PLOT_TYPES_WESL),
    ("/flan/shaders/lib/bindings/plot", PLOT_BINDINGS_WESL),
    ("/flan/shaders/lib/input/plot", PLOT_INPUT_WESL),
    ("/flan/shaders/lib/helpers/plot", PLOT_HELPERS_WESL),
    (
        "/flan/shaders/lib/types/fullscreen_effect",
        FULLSCREEN_EFFECT_TYPES_WESL,
    ),
    (
        "/flan/shaders/lib/bindings/fullscreen_effect",
        FULLSCREEN_EFFECT_BINDINGS_WESL,
    ),
    (
        "/flan/shaders/lib/input/fullscreen_effect",
        FULLSCREEN_EFFECT_INPUT_WESL,
    ),
];

/// Compile a WESL shader to a WGSL string using the in-memory VirtualResolver.
///
/// `stem` is the path relative to the flan crate `src/` dir. That is:
/// `"fullscreen/chromatic-aberration"` , `"2d/plot"`. `src` is the raw WESL
/// bytes for the specific shader after "compiling".
///
/// Returns the compiled WGSL string or an error description if things go sideways.
pub fn compile(stem: &str, src: &str, variant: Variant) -> Result<String, String> {
    // The entry point is setup to look like what bevy apps use aka: "/flan/shaders/<stem>"
    let entry_path = format!("/flan/shaders/{stem}");
    let module_path = mp(&entry_path);

    let resolver = make_resolver(stem, src);

    let mut compiler = wesl::Wesl::new_barebones();
    compiler.use_condcomp(true);
    compiler.use_imports(true);
    compiler.set_feature("UI_MATERIAL", variant.ui_material);
    compiler.set_feature("WEBGL", variant.webgl);
    compiler.set_feature("WGPU_TEST", variant.wgpu_test);
    compiler.use_stripping(false);
    let compiler = compiler.set_custom_resolver(resolver);

    compiler
        .compile(&module_path)
        .map(|m| m.to_string())
        .map_err(|e| {
            format!(
                "WESL compile failed for {stem} ({}, wgpu_test={}): {e:?}",
                variant.dir_name(),
                variant.wgpu_test,
            )
        })
}

/// Patch `@group(from)` -> `@group(to)` in a WGSL string. Kept for any future
/// test that needs group remapping, not sure this will be needed again but who
/// knows.
pub fn patch_group(wgsl: &str, from: u8, to: u8) -> String {
    wgsl.replace(&format!("@group({from})"), &format!("@group({to})"))
}
