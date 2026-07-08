// TODO: I think the uri parsing is better yeeted into lib but I can do that
// later its late enough as it is most of this "refactor" is copy/paste.
//! CLI argument parsing and WASM URI query-string parsing cause I couldn't
//! figure of a better spot for this stuff.
//!
//! Both the native `--flag` interface and the WASM `?key=value` interface
//! produce a `(bool, UiConfig)` tuple consumed by `main()` before the Bevy
//! `App` is constructed. The two code paths share the same validation logic
//! so the URL surface is symmetric with the command-line surface.
use crate::ui;

// TODO: also is there a crate for this crap I forgot to look cause I'm a dumdum
// sometimes and like to here hold my beer crap cause this binary is freaking
// huge as it is more deps won't make it smaller.
/// Minimal percent-decoder for URL query string values.
///
/// Handles `%XX` sequences and `+` -> space. Good enough for IANA timezone
/// names (which may contain `/` encoded as `%2F`) and plain slug strings.
#[cfg(any(target_arch = "wasm32", test))]
pub fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((hi * 16 + lo) as u8 as char);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Decode a raw `?reverie=` query-param value (percent-encoded) into the plain
/// string stored in `UiConfig::initial_reverie`.
#[cfg(any(target_arch = "wasm32", test))]
pub fn reverie_from_query_value(raw: &str) -> String {
    percent_decode(raw)
}

/// Command-line arguments for the native (non-WASM) binary.
///
/// On WASM the equivalent configuration is read from the URL query string;
/// see `parse_wasm_args`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser, Debug)]
#[command(about = "mitchty - just me playing around for funsies", long_version = lib::build_info::VERSTR)]
pub struct Cli {
    /// Enable gamepad support.
    #[arg(long, overrides_with = "no_gamepad")]
    pub with_gamepad: bool,

    /// Disable gamepad support (default).
    ///
    /// Added to avoid a panic under Wine where an Xbox controller confuses a
    /// transitive dependency.
    #[arg(long = "no-gamepad", overrides_with = "with_gamepad")]
    pub no_gamepad: bool,

    /// Open one or more app windows at startup.
    ///
    /// Comma-separated or repeated. Known values: `world-clock`, `recognizer`,
    /// `data-viewer`.
    #[arg(long, value_delimiter = ',', value_name = "APP", action = clap::ArgAction::Append)]
    pub app: Vec<String>,

    /// Open a specific reverie by name at startup.
    ///
    /// Matched case-insensitively so the WASM URI surface stays symmetric with
    /// the command-line args.
    #[arg(long, value_name = "REVERIE")]
    pub reverie: Option<String>,

    /// Override the world-clock timezone list.
    ///
    /// Comma-separated or repeated. Each value must be a valid IANA timezone
    /// name, e.g. `America/New_York`.
    #[arg(long = "tz", value_delimiter = ',', value_name = "TZ", action = clap::ArgAction::Append)]
    pub tz: Vec<String>,

    /// Set up the initial world-clock alarms.
    ///
    /// Format: `IANA_TZ:UTC_SECONDS` or `LABEL:IANA_TZ:UTC_SECONDS`.
    ///
    /// Example: `--alarm America/New_York:1893456000`
    /// Example: `--alarm "Birthday:America/New_York:1893456000"`
    #[arg(long = "alarm", value_name = "[LABEL:]TZ:EPOCH", action = clap::ArgAction::Append)]
    pub alarm: Vec<String>,

    /// Set the initial sort column for the World Clock table.
    ///
    /// Values: `timezone`, `time`, `date`, `offset`, `delta-local`.
    #[arg(long = "sort", value_name = "COLUMN")]
    pub sort: Option<String>,

    /// Set the initial sort direction for the World Clock table.
    ///
    /// Values: `asc` (default), `desc`.
    #[arg(long = "sort-dir", value_name = "DIR")]
    pub sort_dir: Option<String>,

    /// Freeze the World Clock at this UTC moment instead of live time.
    ///
    /// Value is a Unix timestamp (seconds since epoch).
    #[arg(long = "pinned", value_name = "EPOCH")]
    pub pinned: Option<i64>,

    /// Disable one or more plugins at startup for dev builds with dev_build cfg
    /// gate set.
    ///
    /// Comma-separated or repeated. Known values: `postprocess`, `plot`,
    /// `mesheffect`, `prettytext`. Unrecognised names are silently ignored so
    /// typos don't hard-fail the binary.
    ///
    /// Example: `--without-plugins postprocess,mesheffect`
    // TODO: This is just half finished leftovers for me debugging slow memory
    // leaks. Future me will fix it.
    #[cfg(dev_build)]
    #[arg(long = "without-plugins", value_delimiter = ',', value_name = "PLUGIN", action = clap::ArgAction::Append)]
    pub without_plugins: Vec<String>,
}

/// Parse native CLI arguments and return `(enable_gamepad, UiConfig, without_plugins)`.
///
/// The third element is the raw `--without-plugins` list from the CLI. It is
/// always `Vec::new()` in release builds (the flag doesn't exist); the caller
/// is responsible for building a [`bavy::disabled::DisabledPlugins`] resource
/// from it in debug builds.
///
/// Called once at the top of `main()` on non-WASM targets before the Bevy
/// `App` is constructed.
#[cfg(not(target_arch = "wasm32"))]
pub fn parse_native_args() -> (bool, ui::UiConfig, Vec<String>) {
    use clap::Parser;
    use jiff::Timestamp;

    let cli = Cli::parse();
    let mut cfg = ui::UiConfig::default();

    for slug in &cli.app {
        match ui::UiWindow::from_slug(slug) {
            Some(w) => cfg.enable_window(w),
            None => bevy::log::warn!("--app: unknown app {:?} ignoring it", slug),
        }
    }

    if let Some(name) = &cli.reverie {
        cfg.initial_reverie = Some(name.clone());
    }

    // Validate each TZ name against the bundled tz database.
    for tz in &cli.tz {
        let tz = tz.trim();
        if tz.is_empty() {
            continue;
        }
        if jiff_tzdb::available().any(|n| n == tz) {
            cfg.initial_timezones.push(tz.to_string());
        } else {
            bevy::log::warn!("--tz: unknown timezone {:?} ignoring it", tz);
        }
    }

    // Parse --alarm entries.
    for entry in &cli.alarm {
        match ui::world_clock::parse_alarm_entry(entry) {
            Some((secs, tz, label)) => match Timestamp::from_second(secs) {
                Ok(ts) => cfg.initial_alarms.push((ts, tz, label)),
                Err(_) => {
                    bevy::log::warn!("--alarm: epoch out of range in {:?} ignoring it", entry)
                }
            },
            None => bevy::log::warn!(
                "--alarm: expected [LABEL:]TZ:EPOCH format, got {:?} ignoring it",
                entry
            ),
        }
    }

    // Parse --sort column.
    if let Some(col_slug) = &cli.sort {
        use ui::world_clock::SortColumn;
        match SortColumn::from_slug(col_slug.trim()) {
            Some(col) => cfg.initial_sort_col = col,
            None => bevy::log::warn!("--sort: unknown column {:?} ignoring it", col_slug),
        }
    }

    // Parse --sort-dir direction.
    if let Some(dir_slug) = &cli.sort_dir {
        use ui::world_clock::SortDir;
        match SortDir::from_slug(dir_slug.trim()) {
            Some(dir) => cfg.initial_sort_dir = dir,
            None => bevy::log::warn!(
                "--sort-dir: expected asc or desc, got {:?} ignoring it",
                dir_slug
            ),
        }
    }

    // Parse --pinned epoch.
    if let Some(secs) = cli.pinned {
        match Timestamp::from_second(secs) {
            Ok(ts) => cfg.initial_pinned = Some(ts),
            Err(_) => bevy::log::warn!("--pinned: epoch out of range {} ignoring it", secs),
        }
    }

    #[cfg(dev_build)]
    let without_plugins = cli.without_plugins.clone();
    #[cfg(not(dev_build))]
    let without_plugins: Vec<String> = Vec::new();

    (cli.with_gamepad, cfg, without_plugins)
}

/// Parse the browser URL query string and return a populated `UiConfig`.
///
/// The query-string surface is intentionally symmetric with the native CLI so
/// e.g. `?app=world-clock&tz=America/New_York` works the same as
/// `--app world-clock --tz America/New_York`.
///
/// Gamepad support is always disabled on WASM (returns `false`).
#[cfg(target_arch = "wasm32")]
pub fn parse_wasm_args() -> ui::UiConfig {
    use jiff::Timestamp;

    let mut cfg = ui::UiConfig::default();

    let query = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();

    for pair in query.trim_start_matches('?').split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let raw_value = parts.next().unwrap_or("").trim();
        let value = percent_decode(raw_value);
        let value = value.as_str();

        if key.eq_ignore_ascii_case("app") {
            for slug in value.split(',') {
                let slug = slug.trim();
                if slug.is_empty() {
                    continue;
                }
                match ui::UiWindow::from_slug(slug) {
                    Some(w) => cfg.enable_window(w),
                    None => bevy::log::warn!("?app=: unknown app {:?} ignoring it", slug),
                }
            }
        } else if key.eq_ignore_ascii_case("reverie") {
            cfg.initial_reverie = Some(reverie_from_query_value(raw_value));
        } else if key.eq_ignore_ascii_case("tz") {
            for tz in value.split(',') {
                let tz = tz.trim();
                if tz.is_empty() {
                    continue;
                }
                if jiff_tzdb::available().any(|n| n == tz) {
                    cfg.initial_timezones.push(tz.to_string());
                } else {
                    bevy::log::warn!("?tz=: unknown timezone {:?} ignoring it", tz);
                }
            }
        } else if key.eq_ignore_ascii_case("alarm") {
            for entry in value.split(',') {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                match ui::world_clock::parse_alarm_entry(entry) {
                    Some((secs, tz, label)) => match Timestamp::from_second(secs) {
                        Ok(ts) => cfg.initial_alarms.push((ts, tz, label)),
                        Err(_) => bevy::log::warn!(
                            "?alarm=: epoch out of range in {:?} ignoring it",
                            entry
                        ),
                    },
                    None => bevy::log::warn!(
                        "?alarm=: expected [LABEL:]TZ:EPOCH, got {:?} ignoring it",
                        entry
                    ),
                }
            }
        } else if key.eq_ignore_ascii_case("sort") {
            use ui::world_clock::SortColumn;
            match SortColumn::from_slug(value) {
                Some(col) => cfg.initial_sort_col = col,
                None => bevy::log::warn!("?sort=: unknown column {:?} ignoring it", value),
            }
        } else if key.eq_ignore_ascii_case("sort-dir") {
            use ui::world_clock::SortDir;
            match SortDir::from_slug(value) {
                Some(dir) => cfg.initial_sort_dir = dir,
                None => bevy::log::warn!(
                    "?sort-dir=: expected asc or desc, got {:?} ignoring it",
                    value
                ),
            }
        } else if key.eq_ignore_ascii_case("pinned") {
            match value.parse::<i64>() {
                Ok(secs) => match Timestamp::from_second(secs) {
                    Ok(ts) => cfg.initial_pinned = Some(ts),
                    Err(_) => {
                        bevy::log::warn!("?pinned=: epoch out of range {:?} ignoring it", value)
                    }
                },
                Err(_) => bevy::log::warn!(
                    "?pinned=: expected integer epoch, got {:?} ignoring it",
                    value
                ),
            }
        }
    }

    cfg
}

#[cfg(test)]
mod alarm_tests {
    use crate::ui::world_clock::parse_alarm_entry;

    /// Simulate the comma-split loop used by both the native CLI and the WASM
    /// URL parsers.
    fn parse_alarm_value(value: &str) -> Vec<(i64, String, Option<String>)> {
        value
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                parse_alarm_entry(entry)
            })
            .collect()
    }

    #[test]
    fn single_two_part_entry() {
        let results = parse_alarm_value("America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        let (epoch, tz, label) = &results[0];
        assert_eq!(*epoch, 1775012160);
        assert_eq!(tz, "America/Chicago");
        assert_eq!(*label, None);
    }

    #[test]
    fn single_three_part_entry_with_label() {
        let results = parse_alarm_value("Birthday:America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        let (epoch, tz, label) = &results[0];
        assert_eq!(*epoch, 1775012160);
        assert_eq!(tz, "America/Chicago");
        assert_eq!(*label, Some("Birthday".to_string()));
    }

    #[test]
    fn comma_separated_two_entries() {
        let results = parse_alarm_value("America/Chicago:1775012160,America/New_York:1893456000");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, "America/Chicago");
        assert_eq!(results[1].1, "America/New_York");
        assert_eq!(results[0].2, None);
        assert_eq!(results[1].2, None);
    }

    #[test]
    fn comma_separated_mixed_label_and_no_label() {
        let results =
            parse_alarm_value("Birthday:America/Chicago:1775012160,America/New_York:1893456000");
        assert_eq!(results.len(), 2);
        let (_, tz0, lbl0) = &results[0];
        let (_, tz1, lbl1) = &results[1];
        assert_eq!(tz0, "America/Chicago");
        assert_eq!(*lbl0, Some("Birthday".to_string()));
        assert_eq!(tz1, "America/New_York");
        assert_eq!(*lbl1, None);
    }

    #[test]
    fn whitespace_around_entries_is_trimmed() {
        let results = parse_alarm_value("  America/Chicago:1775012160  ,  UTC:1893456000  ");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, "America/Chicago");
        assert_eq!(results[1].1, "UTC");
    }

    #[test]
    fn empty_entry_from_leading_comma_is_skipped() {
        let results = parse_alarm_value(",America/Chicago:1775012160,");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "America/Chicago");
    }

    #[test]
    fn all_whitespace_entry_is_skipped() {
        let results = parse_alarm_value("   ,America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn invalid_entry_is_skipped_valid_ok() {
        let results = parse_alarm_value("notvalid,America/Chicago:1775012160");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "America/Chicago");
    }

    #[test]
    fn all_invalid_entries_returns_empty() {
        let results = parse_alarm_value("bad,alsoBad,stillBad");
        assert!(results.is_empty());
    }

    #[test]
    fn entirely_empty_value_returns_empty() {
        let results = parse_alarm_value("");
        assert!(results.is_empty());
    }
}

#[cfg(test)]
mod percent_decode_tests {
    use super::{percent_decode, reverie_from_query_value};

    #[test]
    fn decode_plain_ascii_unchanged() {
        assert_eq!(percent_decode("lorem_ipsum"), "lorem_ipsum");
    }

    #[test]
    fn decode_slash_uppercase_hex() {
        assert_eq!(percent_decode("namespace%2Ffoo"), "namespace/foo");
    }

    #[test]
    fn decode_slash_lowercase_hex() {
        assert_eq!(percent_decode("namespace%2ffoo"), "namespace/foo");
    }

    #[test]
    fn decode_space_via_plus() {
        assert_eq!(percent_decode("lorem+ipsum"), "lorem ipsum");
    }

    #[test]
    fn decode_space_via_percent20() {
        assert_eq!(percent_decode("lorem%20ipsum"), "lorem ipsum");
    }

    #[test]
    fn decode_empty_string() {
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn decode_truncated_percent_passes_through() {
        assert_eq!(percent_decode("foo%"), "foo%");
        assert_eq!(percent_decode("foo%2"), "foo%2");
    }

    #[test]
    fn decode_mixed() {
        assert_eq!(percent_decode("namespace%2Fbaz_qux"), "namespace/baz_qux");
    }

    #[test]
    fn query_value_plain_slug_unchanged() {
        assert_eq!(reverie_from_query_value("lorem_ipsum"), "lorem_ipsum");
    }

    #[test]
    fn query_value_encoded_slash_decoded() {
        assert_eq!(reverie_from_query_value("namespace%2Ffoo"), "namespace/foo");
    }

    #[test]
    fn query_value_deep_path_decoded() {
        assert_eq!(reverie_from_query_value("a%2Fb%2Fc%2Fdeep"), "a/b/c/deep");
    }

    #[test]
    fn query_value_display_name_with_plus_spaces() {
        assert_eq!(reverie_from_query_value("Lorem+Ipsum"), "Lorem Ipsum");
    }
}
