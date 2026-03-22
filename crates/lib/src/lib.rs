/// Build-time version and environment information, baked in by build.rs.
///
/// Consumers (ma, mitchty, etc.) reference these consts directly, e.g.:
///   #[command(long_version = lib::build_info::VERSTR)]
pub mod build_info {
    /// Short git describe / commit ref baked in at build time.
    pub const GIT_COMMIT: &str = env!("GIT_COMMIT");

    /// rustc version used to compile this build.
    pub const RUSTC_VERSION: &str = env!("RUSTC_VERSION");

    /// Cargo profile (`debug` or `release`).
    pub const BUILD_PROFILE: &str = env!("BUILD_CARGO_PROFILE");

    /// UTC date/time at which this binary was compiled.
    pub const BUILD_DATE: &str = env!("BUILD_DATE");

    /// Full version string suitable for clap's `long_version`.
    /// Format: "<semver> <git-commit> <profile> (rustc <rustc-version>) built <date>"
    pub const VERSTR: &str = const_format::formatcp!(
        "{} {} {} (rustc {}) built {}",
        env!("CARGO_PKG_VERSION"),
        GIT_COMMIT,
        BUILD_PROFILE,
        RUSTC_VERSION,
        BUILD_DATE,
    );
}
