use git_version::git_version;
use jiff::Timestamp;
use rustc_version::version;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=NIX_GIT_REV");

    let build_date = Timestamp::now()
        .strftime("%Y-%m-%d %H:%M:%S UTC")
        .to_string();
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    let rustc_ver = version().expect("failed to get rustc version");
    println!("cargo:rustc-env=RUSTC_VERSION={rustc_ver}");

    let profile = std::env::var("PROFILE").expect("failed to get cargo profile");
    println!("cargo:rustc-env=BUILD_CARGO_PROFILE={profile}");

    // git_version! bakes the git describe output at compile time. Outside of a
    // git repo (e.g. nix sandbox builds), it falls back to "unknown". In that
    // case NIX_GIT_REV must be set by the nix derivation, otherwise we panic to
    // make the missing version info loud rather than silent.
    let unknown = "unknown";
    let nix_rev_missing = "nixunknown";
    let gitver = git_version!(fallback = unknown);
    let nix_rev = std::env::var("NIX_GIT_REV").unwrap_or_else(|_| nix_rev_missing.to_string());

    if gitver == unknown && nix_rev == nix_rev_missing {
        panic!("no git version available and NIX_GIT_REV is not set");
    } else if gitver == unknown || nix_rev != nix_rev_missing {
        println!("cargo:rustc-env=GIT_COMMIT={nix_rev}");
    } else {
        println!("cargo:rustc-env=GIT_COMMIT={gitver}");
    }
}
