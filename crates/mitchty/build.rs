//! Right now this is only used to dynamically generate the Reveries stuff

use std::fmt::Write as FmtWrite;
use std::path::Path;

fn main() {
    let reveries_dir = Path::new("src/assets/reveries");
    println!("cargo:rerun-if-changed={}", reveries_dir.display());

    let mut entries: Vec<(String, String, String)> = Vec::new(); // (key, display, abs_path)

    match collect_reveries(reveries_dir, "", &mut entries) {
        Ok(()) => {}
        Err(e) => {
            println!(
                "cargo:warning=build.rs: cannot scan {}: {} REVERIE_DATA will be empty, may be a bug",
                reveries_dir.display(),
                e
            );
            emit_empty();
            return;
        }
    }

    // Deterministic order regardless of filesystem.
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by Cargo");
    let out_path = Path::new(&out_dir).join("reveries_generated.rs");

    let mut code = String::new();
    writeln!(
        code,
        "pub static REVERIE_DATA: &[(&[&str], &str, &str)] = &["
    )
    .unwrap();

    for (key, display, path) in &entries {
        writeln!(
            code,
            "    (&[{key_lit}], {display_lit}, include_str!({path_lit})),",
            key_lit = quote(key),
            display_lit = quote(display),
            path_lit = quote(path),
        )
        .unwrap();
    }

    writeln!(code, "];").unwrap();

    std::fs::write(&out_path, &code)
        .unwrap_or_else(|e| panic!("build.rs: failed to write {}: {}", out_path.display(), e));
}

/// Recursively walk `dir`, appending one `(key, display, abs_path)` entry to
/// `out` for every `.md` file found at any depth.
///
/// `prefix` is the slash-joined path of ancestor directory stems relative to
/// the reveries root. Empty for stuff in the parent dir.
fn collect_reveries(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, String, String)>,
) -> std::io::Result<()> {
    let mut items: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    items.sort_by_key(|e| e.file_name());

    for item in items {
        let path = item.path();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_file() {
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let key = if prefix.is_empty() {
                stem.to_owned()
            } else {
                format!("{}/{}", prefix, stem)
            };
            out.push((key, to_title_case(stem), abs_path(&path)));
        } else if meta.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let new_prefix = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{}/{}", prefix, name)
            };
            collect_reveries(&path, &new_prefix, out)?;
        }
    }
    Ok(())
}

fn emit_empty() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by Cargo");
    let out_path = Path::new(&out_dir).join("reveries_generated.rs");
    std::fs::write(
        &out_path,
        "pub static REVERIE_DATA: &[(&[&str], &str, &str)] = &[];\n",
    )
    .unwrap_or_else(|e| panic!("build.rs: failed to write empty generated file: {}", e));
}

/// Convert `snake_case` or `kebab-case` names to `Title Case With Spaces` for
/// stuff in mitchty to abuse.
fn to_title_case(stem: &str) -> String {
    stem.split(['_', '-'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Absolute path normalized to forward slashes for `include_str!` abuse.
fn abs_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .expect("build.rs: cannot determine current working dir")
                .join(path)
        })
        .to_str()
        .expect("build.rs: path is not valid UTF-8")
        .replace('\\', "/")
}

/// Wrap some string `s` in double-quotes for embedding as a Rust string literal
/// for include! embeds.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}
