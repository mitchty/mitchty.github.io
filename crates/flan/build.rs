// build.rs is mostly just here to generate constants for use in wgsl-rs shaders
// and shader handle uuids now.
//   constants.rs:       shared numeric constants for wgsl-rs shaders
//   shader_handles.rs:  u128 UUIDv5 constants and handle functions for every shader
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Max number of plot points stored in the texture-variant uniform buffer.
const MAX_PLOT_POINTS: usize = 512;

/// How many DataFrame rows for a window for the plot shader per sync.
const PLOT_WINDOW_SIZE: usize = 200;

/// Width of the slug data textures used on the WebGL path.
const SLUG_TEX_WIDTH: u32 = 2048;

/// Number of GPU-side plot points for the stats overlay sparkline.
const STATS_OVERLAY_POINT_COUNT: usize = 256;

/// Backing data for the internal history for fps basically with the stats
/// overlay. Each 10 point quantization = what ^^^ ends up being. Its an old
/// smoothing function but it checks out.
const STATS_OVERLAY_HISTORY_SIZE: usize = 2560;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set in env"));

    write_constants(&out_dir);
    write_shader_handles(&out_dir);
}

/// Emit a `u128` constant and its `pub fn *_shader_handle()` function for the shader name.
fn write_shader_handles(out_dir: &Path) {
    // Fixed namespace for the flan crate, for future possible usage where I
    // persist assets to disk. Built from uuidv4 at some point and hopefully it
    // never changes but I dunno I can't predict the future.
    let ns = Uuid::parse_str("c7e1a290-4f38-5b2d-9e8a-1d6c047b3f52")
        .expect("hard-coded namespace UUID must be valid");

    // TODO: maybe render feature just goes away? or if I can just detect if i'm
    // running in a cargo test/nextest invocation I can gate the wgpu and test
    // related crap there.
    let shaders: &[(&str, &str)] = &[
        ("plot_default", ""),
        ("plot_texture", ""),
        ("slug_text_default", ""),
        ("slug_text_texture", ""),
        ("slug_text3d_default", ""),
        ("slug_text3d_texture", ""),
        ("stats_overlay_default", ""),
        ("stats_overlay_texture", ""),
        ("chromatic_aberration", ""),
        ("vhs_effect", ""),
        ("em_interference", ""),
        ("oil_painting", ""),
        ("edge_cartoon", ""),
        ("cartoon_filter", ""),
    ];

    let mut out = String::from("// Built by build.rs, don't sit on me!\n");

    for (shader_name, cfg) in shaders {
        let const_name = shader_name.to_uppercase();
        let uuid = Uuid::new_v5(&ns, shader_name.as_bytes());
        let value = uuid.as_u128();

        if !cfg.is_empty() {
            out.push_str(&format!("{cfg}\n"));
        }
        out.push_str(&format!(
            "pub const {const_name}_SHADER_HANDLE: u128 = 0x{value:032x};\n"
        ));

        if !cfg.is_empty() {
            out.push_str(&format!("{cfg}\n"));
        }
        out.push_str(&format!(
            r#"pub fn {shader_name}_shader_handle() -> bevy::asset::Handle<bevy::shader::Shader> {{
   bevy::asset::Handle::from(bevy::asset::uuid::Uuid::from_u128({const_name}_SHADER_HANDLE))
}}"#
        ));
    }

    fs::write(out_dir.join("shader_handles.rs"), out)
        .expect("build.rs: could not write shader_handles.rs");
}

fn write_constants(out_dir: &Path) {
    let content = format!(
        r#"

pub const MAX_PLOT_POINTS: usize = {MAX_PLOT_POINTS};
pub const PLOT_WINDOW_SIZE: usize = {PLOT_WINDOW_SIZE};
pub const SLUG_TEX_WIDTH: u32 = {SLUG_TEX_WIDTH};
pub const STATS_OVERLAY_POINT_COUNT: usize = {STATS_OVERLAY_POINT_COUNT};
pub const STATS_OVERLAY_HISTORY_SIZE: usize = {STATS_OVERLAY_HISTORY_SIZE};
"#
    );
    fs::write(out_dir.join("constants.rs"), content)
        .expect("build.rs could not write constants.rs file");
}
