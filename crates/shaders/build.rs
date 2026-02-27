// build.rs for the `shaders` crate.
//
// Dumps out src/shaders/*.wesl for top-level shaders and src/shaders/lib/ for
// shared helper modules, compiles each shader into every required variant using
// WESL conditional-compilation feature flags, then generates
// $OUT_DIR/shaders.rs which is include!()-ed by src/lib.rs for actual usage by
// other crates (flan mostly for now, mitchty later)
//
// Directory layout:
//   src/shaders/
//     <shader>.wesl              ← top-level shaders (compiled 4× each)
//     lib/
//       bindings/<name>.wesl     ← binding declarations
//       helpers/<name>.wesl      ← utility functions
//       input/<name>.wesl        ← FragmentInput struct definitions
//       types/<name>.wesl        ← shared type/struct definitions
//
// Helpers are imported in shaders roughly like laid out ^^^^^^^ to make it
// easier for future jerk me to understand cause I WILL FORGET HOW THIS CRAP
// WORKS. Future mitch be happy I documented this crap for once.
//   import package::lib::bindings::<name>::{…};
//   import package::lib::helpers::<name>::{…};
//   import package::lib::input::<name>::{…};
//   import package::lib::types::<name>::{…};
//
// Output per shader, note bevy is assumed for now, I might make a wgpu option at some point:
//   bevy/default/material/<stem>.wgsl   {}
//   bevy/default/ui/<stem>.wgsl         {UI_MATERIAL}
//   bevy/webgl/material/<stem>.wgsl     {WEBGL}
//   bevy/webgl/ui/<stem>.wgsl           {UI_MATERIAL, WEBGL}
//
// Generated file contains crap that the bevy plugin abuses and exposes for callers:
//   pub const BEVY_DEFAULT_MATERIAL_<STEM> : Handle<Shader> = uuid_handle!("...");
//   pub const BEVY_DEFAULT_UI_<STEM>       : Handle<Shader> = uuid_handle!("...");
//   #[cfg(feature = "webgl")]
//   pub const BEVY_WEBGL_MATERIAL_<STEM>   : Handle<Shader> = uuid_handle!("...");
//   #[cfg(feature = "webgl")]
//   pub const BEVY_WEBGL_UI_<STEM>         : Handle<Shader> = uuid_handle!("...");
//
// The handles are uuidv5 so that they are deterministic across rebuilds and
// don't cause a ton of crap to rebuild needlessly. I think I didn't test this
// out that much.
use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

// "namespace" for the shaders, fixed dummy uuid. Don't change it in case anyone
// depends on Handle uuids being deterministic. Nobody should but just in case.
const SHADER_NS_STR: &str = "123e4567-e89b-12d3-a456-426614174000";

// crap in src/shaders/lib to setup in the resulting compiled wgsl shader.
const LIB_CATEGORIES: &[&str] = &["bindings", "helpers", "input", "types"];

/// Scan src/shaders/lib/<category>/*.wesl and return a list of
/// (module_path_key, source) pairs, where module_path_key is the string
/// that must be registered in the VirtualResolver for the shader package
/// `shader_stem`, e.g. `"plot::lib::bindings::plot"`.
fn collect_helpers(shaders_src: &Path, shader_stem: &str) -> Vec<(String, String)> {
    let lib_dir = shaders_src.join("lib");
    let mut helpers = Vec::new();

    for category in LIB_CATEGORIES {
        let cat_dir = lib_dir.join(category);
        let entries = match fs::read_dir(&cat_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "wesl") {
                let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
                let src = fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("couldn't read {}", path.display()));
                // Mirrors the import path used in .wesl files:
                //   import package::lib::<category>::<name>::{…}
                // which is equivalent to
                //   ModulePath { origin: Package(shader_stem), components: ["lib", category, name] }
                let key = format!("{shader_stem}::lib::{category}::{name}");
                helpers.push((key, src));
            }
        }
    }

    helpers.sort_by(|a, b| a.0.cmp(&b.0));
    helpers
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let shaders_src = manifest_dir.join("src/shaders");
    let webgl = env::var("CARGO_FEATURE_WEBGL").is_ok();

    println!("cargo:rerun-if-changed=src/shaders");

    // TODO: When bevy kills webgl2 support this probably becomes waaaay simpler.
    //
    // Variants: (backend_dir, type_dir, UI_MATERIAL, WEBGL, needs_webgl_feature)
    //  output path ~= bevy/<backend_dir>/<type_dir>/<stem>.wgsl
    //  output var  ~=  BEVY_<BACKEND_DIR>_<TYPE_DIR>_<STEM>
    let mut variants: Vec<(&str, &str, bool, bool, bool)> = vec![
        ("default", "material", false, false, false),
        ("default", "ui", true, false, false),
    ];
    if webgl {
        variants.push(("webgl", "material", false, true, true));
        variants.push(("webgl", "ui", true, true, true));
    }

    let ns = uuid::Uuid::parse_str(SHADER_NS_STR).expect("namespace uuid is wack bra");

    // Grab all the crap in src/shaders/*.wesl, nothing under however.
    let mut shaders: Vec<(String, PathBuf)> = fs::read_dir(&shaders_src)
        .expect("src/shaders not found — did you add .wesl files there?")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wesl"))
        .map(|p| {
            let stem = p.file_stem().unwrap().to_str().unwrap().to_owned();
            (stem, p)
        })
        .collect();
    shaders.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic order

    // Helper: build a VirtualResolver with all lib/* helpers pre-registered
    // as nested submodules of the shader being compiled.
    //
    // Shaders use `import package::lib::bindings::plot::` which resolves to
    // `ModulePath { origin: Package(stem), components: ["lib","bindings","plot"] }`.
    // We register each helper as `"<stem>::lib::<category>::<name>"`.
    let make_resolver = |stem: &str, src: &str| -> wesl::VirtualResolver {
        let mut r = wesl::VirtualResolver::new();
        // Register all lib/* helpers under this shader's package.
        for (key, helper_src) in collect_helpers(&shaders_src, stem) {
            let mp: wesl::ModulePath = key
                .parse()
                .unwrap_or_else(|_| panic!("invalid module path: {key}"));
            r.add_module(mp, helper_src.into());
        }
        // Register the shader being compiled as the package root.
        let mp: wesl::ModulePath = stem
            .parse()
            .unwrap_or_else(|_| panic!("invalid module path: {stem}"));
        r.add_module(mp, src.to_owned().into());
        r
    };

    // TODO: I should really find out if there is a better way to build a rust dag
    // But old age and treachery wins over everything.
    let mut registry = String::new();
    writeln!(
        registry,
        "// This file is autogenerated by shaders/build.rs, caveat editor >.<"
    )
    .unwrap();
    writeln!(registry).unwrap();

    let mut register_fn =
        String::from("pub(crate) fn _register_shaders(app: &mut bevy::app::App) {\n");

    for (stem, path) in &shaders {
        let src = fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("could not read {}", path.display()));

        for (backend_dir, type_dir, ui_material, webgl_flag, needs_webgl_feature) in &variants {
            let out_variant_dir = out_dir.join("bevy").join(backend_dir).join(type_dir);
            fs::create_dir_all(&out_variant_dir).unwrap();
            let out_file = out_variant_dir.join(format!("{stem}.wgsl"));

            let module_path: wesl::ModulePath = stem
                .parse()
                .expect("stem does not map to a WESL module path");

            let resolver = make_resolver(stem, &src);

            let mut compiler = wesl::Wesl::new_barebones();
            compiler.use_condcomp(true);
            compiler.use_imports(true);
            compiler.set_feature("UI_MATERIAL", *ui_material);
            compiler.set_feature("WEBGL", *webgl_flag);
            compiler.use_stripping(false);
            let compiler = compiler.set_custom_resolver(resolver);

            let result = compiler.compile(&module_path).unwrap_or_else(|e| {
                panic!("compile failed for wesl {stem} variant {backend_dir}/{type_dir}: {e:?}")
            });

            fs::write(&out_file, result.to_string()).unwrap();

            // path bevy/<backend>/<type>/<stem>.wgsl ==
            //      BEVY_DEFAULT_MATERIAL_PLOT
            //      BEVY_WEBGL_UI_REFERENCE
            //      usw, etc...
            let const_name = format!(
                "BEVY_{}_{}_{}",
                backend_dir.to_uppercase().replace('-', "_"),
                type_dir.to_uppercase().replace('-', "_"),
                stem.to_uppercase().replace('-', "_"),
            );

            // This is that dummy uuid const above and "stem:backend/type"
            // appended to it so that everything built is deterministic.
            let uuid =
                uuid::Uuid::new_v5(&ns, format!("{stem}:{backend_dir}/{type_dir}").as_bytes());

            let cfg_attr = if *needs_webgl_feature {
                "    #[cfg(feature = \"webgl\")]\n"
            } else {
                ""
            };
            writeln!(
                registry,
                "{cfg_attr}pub const {const_name}: bevy::asset::Handle<Shader> = \
                 bevy::asset::uuid_handle!(\"{uuid}\");",
            )
            .unwrap();

            let cfg_inner = if *needs_webgl_feature {
                "    #[cfg(feature = \"webgl\")]\n"
            } else {
                ""
            };
            writeln!(
                register_fn,
                "{cfg_inner}    bevy::asset::load_internal_asset!(app, {const_name}, \
                 concat!(env!(\"OUT_DIR\"), \"/bevy/{backend_dir}/{type_dir}/{stem}.wgsl\"), Shader::from_wgsl);",
            )
            .unwrap();
        }

        writeln!(registry).unwrap();
    }

    register_fn.push_str("}\n");
    registry.push_str(&register_fn);

    fs::write(out_dir.join("shaders.rs"), &registry).unwrap();
}
