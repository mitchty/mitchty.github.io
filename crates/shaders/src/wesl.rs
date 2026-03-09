// wesl -> wgsl compile crap.
//
// Here to start making unit testing of stuff easier.
//
// This is "kinda" build.rs but only for runtime testing of shaders.
//
// I don't know what should be DRY, so its WET. I might not need to add much
// more to this stuff anyway. Famous last words of not knowing whats needed.
// WESL → WGSL compilation helpers.
//
// I might consider refactoring build.rs off/out of this too.
//
// Note abusing VirtualResolvers so wesl shaders can "just" import stuff from
// `src/shaders/lib/...` directly.

// For now WET, not sure its worth the effort to make this cute/dynamic and want
// to commit this nonsense and get cracking on building plotting shaders so I
// can feed em from polars dataframes directly. Thats the real reason for this
// whole side quest.
const PLOT_TYPES_WESL: &str = include_str!("shaders/lib/types/plot.wesl");
const PLOT_BINDINGS_WESL: &str = include_str!("shaders/lib/bindings/plot.wesl");
const PLOT_INPUT_WESL: &str = include_str!("shaders/lib/input/plot.wesl");
const PLOT_HELPERS_WESL: &str = include_str!("shaders/lib/helpers/plot.wesl");

const CHROMATIC_ABERRATION_TYPES_WESL: &str =
    include_str!("shaders/lib/types/chromatic_aberration.wesl");
const CHROMATIC_ABERRATION_BINDINGS_WESL: &str =
    include_str!("shaders/lib/bindings/chromatic_aberration.wesl");
const CHROMATIC_ABERRATION_INPUT_WESL: &str =
    include_str!("shaders/lib/input/chromatic_aberration.wesl");

/// wesl variant to build, also has `wgpu_test` for a "simple" non bevy variant
/// that isn't built at compile time, only used for testing. The wgpu variant
/// just throws everything into @group(0). I got sick of trying to render whate
/// bevy considers a material of any sort in wgpu in the layout differences.
///
/// Did this more so I don't need a custom vertex stage in wgpu. Winter mitch
/// thing to tackle.
///
/// Should probably become an Enum of Material, UIMaterial, Webgl, Wgpu or whatever.
/// TODO: That is a future sucker mitch problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    pub ui_material: bool,
    pub webgl: bool,
    /// Shaders that opt in to this flag (via `@if(WGPU_TEST)`) can provide
    /// a simplified layout that the headless test harness always knows how to
    /// drive — e.g. forcing all bindings to @group(0) and using a minimal
    /// FragmentInput.
    ///
    /// Only unit tests use this simple variant.
    pub wgpu_test: bool,
}

impl Variant {
    /// `@group(2)` params, storage-buffer points e.g. bevy `Material`
    pub const MATERIAL: Self = Self {
        ui_material: false,
        webgl: false,
        wgpu_test: false,
    };
    // Corresponds to bevy UIMaterial
    /// `@group(1)` params, storage-buffer points e.g. bevy `UiMaterial`
    pub const UI: Self = Self {
        ui_material: true,
        webgl: false,
        wgpu_test: false,
    };
    // These really are the same as ^^^ but just with padding for webgl to work.
    /// `@group(2)` params, uniform-buffer points e.g. bevy `Material` padded to 16byte alignment
    pub const WEBGL: Self = Self {
        ui_material: false,
        webgl: true,
        wgpu_test: false,
    };
    /// `@group(1)` params, uniform-buffer points e.g. bevy `UIMaterial` padded to 16byte alignment
    pub const WEBGL_UI: Self = Self {
        ui_material: true,
        webgl: true,
        wgpu_test: false,
    };

    /// Unit test variant for something like bevy `Material`, used in `build.rs`
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
    // Note webgl2 needs inputs as uniform buffers, no storagebuffers possible
    // here. Webgpu is ok with storage buffers but its slow.
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

    /// Output dir for `build.rs` to abuse to store things into. Note wgpu
    /// doesn't have reference images, build.rs doesn't emit anything there.
    pub fn dir_name(self) -> &'static str {
        match (self.ui_material, self.webgl) {
            (false, false) => "material",
            (true, false) => "ui",
            (false, true) => "webgl",
            (true, true) => "webgl-ui",
        }
    }
}

/// Builds a wesl `VirtualResolver` with all the lib crap registered under the
/// right names so input shaders are simple af/easy to brain.
///
/// THAT MEANS import of `package::lib::bindings::blah` resolves to the right
/// wesl::ModulePath and the wesl compiler does the right thing.
///
/// Aka import of `package::lib::bindings::plot` as package `stem` becomes:
///  `ModulePath { origin: Package(stem), components: ["lib","bindings","plot"]
///  }`
///
/// Each helper is equivalent to `"<stem>::lib::<category>::<name>"` so the wesl
/// `VirtualResolver` lookup works.
fn make_resolver(stem: &str) -> wesl::VirtualResolver<'_> {
    let mut r = wesl::VirtualResolver::new();
    for (suffix, src) in HELPERS {
        let key = format!("{stem}::{suffix}");
        r.add_module(key.parse().unwrap(), (*src).to_owned().into());
    }
    r
}

// TODO: This could probably do with making things dynamic when I add more
// things than plotting.
const HELPERS: &[(&str, &str)] = &[
    ("lib::types::plot", PLOT_TYPES_WESL),
    ("lib::bindings::plot", PLOT_BINDINGS_WESL),
    ("lib::input::plot", PLOT_INPUT_WESL),
    ("lib::helpers::plot", PLOT_HELPERS_WESL),
    (
        "lib::types::chromatic_aberration",
        CHROMATIC_ABERRATION_TYPES_WESL,
    ),
    (
        "lib::bindings::chromatic_aberration",
        CHROMATIC_ABERRATION_BINDINGS_WESL,
    ),
    (
        "lib::input::chromatic_aberration",
        CHROMATIC_ABERRATION_INPUT_WESL,
    ),
];

/// "compiles" wesl with all the library resolver stuff setup to a wgsl file
///
/// `stem` must be a valid wesl module path e.g. `plot`
/// `src` is that raw wesl file data
///
/// Returns the wgsl shader or string of whatever failed. Not the best interface...
pub fn compile(stem: &str, src: &str, variant: Variant) -> Result<String, String> {
    let module_path: wesl::ModulePath = stem
        .parse()
        .map_err(|e| format!("invalid module path {stem:?}: {e}"))?;

    let mut resolver = make_resolver(stem);
    resolver.add_module(module_path.clone(), src.to_owned().into());

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

/// Patches `@group(from)` to `@group(to)` in a wgsl string source. Only useful
/// to hack changing a group input to something else. Nobody but me should need
/// this.
///
/// Here mostly to yeet stuff to `@group(0)` for unit tests
pub fn patch_group(wgsl: &str, from: u8, to: u8) -> String {
    wgsl.replace(&format!("@group({from})"), &format!("@group({to})"))
}
