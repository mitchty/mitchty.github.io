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

    let unknown = "unknown";
    let nix_rev_missing = "nixunknown";
    let gitver = git_version!(args = ["--always", "--dirty=-modified"], fallback = unknown);
    let nix_rev = std::env::var("NIX_GIT_REV").unwrap_or_else(|_| nix_rev_missing.to_string());

    if gitver == unknown && nix_rev == nix_rev_missing {
        panic!("no git version available and NIX_GIT_REV is not set");
    } else if gitver == unknown || nix_rev != nix_rev_missing {
        // When built via Nix, use NIX_GIT_REV, should always be clean or
        // treated that way at least even if the original git checkout was
        // dirty as nix copies the local checkout anyway.
        println!("cargo:rustc-env=GIT_COMMIT={nix_rev}");
        println!("cargo:rustc-env=GIT_DIRTY=false");
    } else {
        // Local git build strip the -modified suffix for the commit. Use dirty
        // bool for knowing if its a wip hold my beer commit or not.
        let dirty = gitver.ends_with("-modified");
        let commit = gitver.trim_end_matches("-modified");
        println!("cargo:rustc-env=GIT_COMMIT={commit}");
        println!("cargo:rustc-env=GIT_DIRTY={dirty}");
    }
}
