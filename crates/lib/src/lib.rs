pub mod img;
pub mod npz;

/// Shared build-time version and environment information for all crates.
pub mod build_info {
    /// Git repo this is nominally built from for link usage.
    pub const GIT_REPO: &str = "https://github.com/mitchty/mitchty.github.io";

    /// Short git commit.
    pub const GIT_COMMIT: &str = env!("GIT_COMMIT");

    /// rustc version that was used for compilation. Not sure its useful but eh.
    pub const RUSTC_VERSION: &str = env!("RUSTC_VERSION");

    /// Cargo profile used for the build.
    pub const BUILD_PROFILE: &str = env!("BUILD_CARGO_PROFILE");

    /// Whether or not the git checkout was dirty/had uncommitted changes.
    pub const GIT_DIRTY: bool = env!("GIT_DIRTY").as_bytes()[0] == b't';

    /// UTC date/time for compilation.
    pub const BUILD_DATE: &str = env!("BUILD_DATE");

    /// Full version string suitable for clap's `long_version`.
    /// Format: "semver git-commit profile rustc rustc-version built date"
    pub const VERSTR: &str = const_format::formatcp!(
        "{} {} {} rustc {} built {}",
        env!("CARGO_PKG_VERSION"),
        GIT_COMMIT,
        BUILD_PROFILE,
        RUSTC_VERSION,
        BUILD_DATE,
    );
}
