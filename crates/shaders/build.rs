// build.rs for the `shaders` crate.
//
// Scans src/shaders/<kind>/*.wesl for top-level shaders and compiles each
// shader into every required variant using WESL conditional-compilation feature
// flags, then generates $OUT_DIR/shaders.rs which is include!()-ed by
// src/lib.rs for actual usage by other crates (flan mostly for now, mitchty later).
//
// Shader kinds are just pseudo namespace subdirs under src/shaders/ currently
// 2d only really. But 3d and compute or other shader kinds could be added. Not
// sure yet on how I want to lay this junk out yet.
//
// Directory layout:
//   src/shaders/
//     2d/
//       <shader>.wesl            = 2d fragment shaders
//     3d/                        = populated when 3d shaders land
//       <shader>.wesl
//     lib/                       = shared helpers across all shader kinds
//       bindings/<name>.wesl     = binding declarations
//       helpers/<name>.wesl      = utility functions
//       input/<name>.wesl        = FragmentInput struct definitions
//       types/<name>.wesl        = shared type/struct definitions
//
// Helpers are imported in shaders roughly like laid out ^^^^^^^ to make it
// easier for future jerk me to understand cause I WILL FORGET HOW THIS CRAP
// WORKS. Future mitch be happy I documented this crap for once.
//   import package::lib::bindings::<name>::{...};
//   import package::lib::helpers::<name>::{...};
//   import package::lib::input::<name>::{...};
//   import package::lib::types::<name>::{...};
//
// Output per shader, note bevy is assumed for now, I might make a wgpu option at some point:
//   bevy/default/material/2d/<stem>.wgsl   {}
//   bevy/default/ui/2d/<stem>.wgsl         {UI_MATERIAL}
//   bevy/webgl/material/2d/<stem>.wgsl     {WEBGL}
//   bevy/webgl/ui/2d/<stem>.wgsl           {UI_MATERIAL, WEBGL}
//
// Generated file contains crap that the bevy plugin abuses and exposes for callers:
//   pub const BEVY_DEFAULT_MATERIAL_2D_<STEM> : Handle<Shader> = uuid_handle!("...");
//   pub const BEVY_DEFAULT_UI_2D_<STEM>       : Handle<Shader> = uuid_handle!("...");
//   #[cfg(feature = "webgl")]
//   pub const BEVY_WEBGL_MATERIAL_2D_<STEM>   : Handle<Shader> = uuid_handle!("...");
//   #[cfg(feature = "webgl")]
//   pub const BEVY_WEBGL_UI_2D_<STEM>         : Handle<Shader> = uuid_handle!("...");
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
// lib is shared across all kinds of shaders.
const LIB_CATEGORIES: &[&str] = &["bindings", "helpers", "input", "types"];

// Shader kinds in src/shaders/. 2d for flat/ui shaders, 3d for mesh-space
// shaders, fullscreen for post-process effects that run over the entire
// rendered framebuffer. Compute will happen at some point. "when its done"
const SHADER_KINDS: &[&str] = &["2d", "3d", "fullscreen"];

/// Scan src/shaders/lib/<category>/*.wesl and return a list of
/// (module_path_key, source) pairs, where module_path_key is the string
/// that must be registered in the VirtualResolver for the shader package
/// `shader_stem`, e.g. `"plot::lib::bindings::plot"`.
///
/// `shaders_src` is the path to `src/shaders/` (NOT the kind subdir).
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
                //   import package::lib::<category>::<name>::{...}
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
    //  output path ~= bevy/<backend_dir>/<type_dir>/<kind>/<stem>.wgsl
    //  output var  ~= BEVY_<BACKEND_DIR>_<TYPE_DIR>_<KIND>_<STEM>
    let mut variants: Vec<(&str, &str, bool, bool, bool)> = vec![
        ("default", "material", false, false, false),
        ("default", "ui", true, false, false),
    ];
    if webgl {
        variants.push(("webgl", "material", false, true, true));
        variants.push(("webgl", "ui", true, true, true));
    }

    let ns = uuid::Uuid::parse_str(SHADER_NS_STR).expect("namespace uuid is wack bra");

    // Helper function that builds a VirtualResolver with all lib/* helpers
    // pre-registered as nested submodules of the shader being compiled.
    //
    // lib is functions that could be used anywhere
    //
    // Shaders use `import package::lib::bindings::plot::` which resolves to
    // `ModulePath { origin: Package(stem), components: ["lib","bindings","plot"] }`.
    // We register each helper as `"<stem>::lib::<category>::<name>"`.
    let make_resolver = |stem: &str, src: &str| -> wesl::VirtualResolver {
        let mut r = wesl::VirtualResolver::new();
        for (key, helper_src) in collect_helpers(&shaders_src, stem) {
            let mp: wesl::ModulePath = key
                .parse()
                .unwrap_or_else(|_| panic!("invalid module path: {key}"));
            r.add_module(mp, helper_src.into());
        }
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

    // Walk each shader kind dir, e.g. 2d, 3d, ... Missing kinds aren't
    // important enough to error out on yet.
    for kind in SHADER_KINDS {
        let kind_src = shaders_src.join(kind);

        // Find all *.wesl files directly under src/shaders/<kind>/.
        let mut shaders: Vec<(String, PathBuf)> = match fs::read_dir(&kind_src) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "wesl"))
                .map(|p| {
                    let stem = p.file_stem().unwrap().to_str().unwrap().to_owned();
                    (stem, p)
                })
                .collect(),
            Err(_) => continue,
        };
        shaders.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic order

        for (stem, path) in &shaders {
            let src = fs::read_to_string(path)
                .unwrap_or_else(|_| panic!("could not read {}", path.display()));

            for (backend_dir, type_dir, ui_material, webgl_flag, needs_webgl_feature) in &variants {
                let out_variant_dir = out_dir
                    .join("bevy")
                    .join(backend_dir)
                    .join(type_dir)
                    .join(kind);
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
                            panic!(
                                "compile failed for wesl {kind}/{stem} variant {backend_dir}/{type_dir}: {e:?}"
                            )
                        });

                fs::write(&out_file, result.to_string()).unwrap();

                // path bevy/<backend>/<type_dir>/<kind>/<stem>.wgsl maps to e.g.:
                //      BEVY_DEFAULT_MATERIAL_2D_PLOT
                //      BEVY_WEBGL_UI_2D_REFERENCE
                let kind_upper = kind.to_uppercase().replace('-', "_");
                let const_name = format!(
                    "BEVY_{}_{}_{}_{}",
                    backend_dir.to_uppercase().replace('-', "_"),
                    type_dir.to_uppercase().replace('-', "_"),
                    kind_upper,
                    stem.to_uppercase().replace('-', "_"),
                );

                // UUID is built off of "kind/stem:backend/type_dir" for
                // stability based on actual path name for the Handle
                let uuid = uuid::Uuid::new_v5(
                    &ns,
                    format!("{kind}/{stem}:{backend_dir}/{type_dir}").as_bytes(),
                );

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
                             concat!(env!(\"OUT_DIR\"), \"/bevy/{backend_dir}/{type_dir}/{kind}/{stem}.wgsl\"), Shader::from_wgsl);",
                        )
                        .unwrap();
            }

            writeln!(registry).unwrap();
        }
    }

    register_fn.push_str("}\n");
    registry.push_str(&register_fn);

    fs::write(out_dir.join("shaders.rs"), &registry).unwrap();
}
