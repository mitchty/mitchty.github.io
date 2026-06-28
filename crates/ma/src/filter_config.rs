//! YAML-based filter configuration for `ma convert --filter-config`.
//!
//! Keeps the long `--filter` / `--filter-name` argument chains out of shell
//! history and into a checked-in file that is easy to edit between runs.
//!
//! # YAML format
//!
//! ```yaml
//! # Characters to exclude verbatim (each entry must be a single Unicode char)
//! chars:
//!   - "!"
//!   - "~"
//!   - ","
//!
//! # Unicode-name substrings to exclude (case-insensitive, substring match)
//! names:
//!   - halfwidth
//!   - fullwidth
//!   - katakana iteration
//!   - middle dot
//!   - space
//!
//! # Characters to include verbatim (whitelist; empty = include everything)
//! include_chars:
//!   - "ア"
//!
//! # Unicode-name substrings to include as a whitelist; empty = include everything
//! # When non-empty a character must match at least one entry to survive.
//! # Composes with the blacklist: blacklisted chars are still excluded even if
//! # they would match an include_names entry.
//! include_names:
//!   - katakana
//!
//! # Merge halfwidth katakana into fullwidth standard forms
//! merge_halfwidth: true
//! ```
//!
//! All keys are optional and default to empty lists/true. Any additional chars
//! or names supplied via CLI `--filter` / `--filter-name` / `--include` /
//! `--include-name` are **appended** to the values loaded from the file so the
//! two sources compose rather than compete.

use std::path::Path;

use serde::Deserialize;

/// Filter configuration loaded from a YAML file.
///
/// All fields are optional in the file, absent keys deserialize to empty `Vec`s
/// or the documented default.
///
/// ## Blacklist vs whitelist
///
/// `chars` / `names` are a blacklist: matching characters are excluded.
///
/// `include_chars` / `include_names` are a whitelist: when either list is
/// non-empty, a character must match at least one entry to survive. Characters
/// that pass neither are dropped. The blacklist is applied first as a character
/// can be excluded by the blacklist even when it would satisfy the whitelist.
#[derive(Debug, Default, Deserialize)]
pub struct FilterConfig {
    /// Individual Unicode characters to exclude as a blacklist.
    #[serde(default)]
    pub chars: Vec<String>,

    /// Unicode-name substrings to exclude as a blacklist, case-insensitive.
    #[serde(default)]
    pub names: Vec<String>,

    /// Individual Unicode characters to keep as a whitelist.
    ///
    /// When non-empty, only characters listed here (or matching
    /// [`include_names`](Self::include_names)) survive. Empty means "keep
    /// everything that the blacklist doesn't drop".
    #[serde(default)]
    pub include_chars: Vec<String>,

    /// Unicode-name substrings to keep as a whitelist, case-insensitive.
    ///
    /// When non-empty (together with `include_chars`), a character must match
    /// at least one entry here or in `include_chars` to survive.
    /// Example: `["katakana"]` keeps all katakana characters.
    #[serde(default)]
    pub include_names: Vec<String>,

    /// Merge halfwidth katakana (U+FF65..U+FF9F) into their standard fullwidth
    /// katakana forms. Default: true.
    #[serde(default = "default_merge_halfwidth")]
    pub merge_halfwidth: bool,
}

fn default_merge_halfwidth() -> bool {
    true
}

impl FilterConfig {
    /// Load a [`FilterConfig`] from a YAML file at `path`.
    ///
    /// Returns an error if the file cannot be read or fails to parse.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read filter config {}: {e}", path.display()))?;
        let cfg: FilterConfig = serde_yaml::from_str(&src)
            .map_err(|e| format!("cannot parse filter config {}: {e}", path.display()))?;
        Ok(cfg)
    }

    /// Resolve `chars` strings into actual `char` values, warning and
    /// skipping any token that is not exactly one Unicode scalar.
    pub fn parse_chars(&self) -> Vec<char> {
        Self::parse_char_vec(&self.chars)
    }

    /// Resolve `include_chars` strings into actual `char` values.
    pub fn parse_include_chars(&self) -> Vec<char> {
        Self::parse_char_vec(&self.include_chars)
    }

    fn parse_char_vec(v: &[String]) -> Vec<char> {
        v.iter()
            .filter_map(|t| {
                let mut cs = t.chars();
                let ch = cs.next()?;
                if cs.next().is_some() {
                    tracing::warn!(
                        token = t.as_str(),
                        "filter-config: ignoring chars entry that is not a single character"
                    );
                    return None;
                }
                Some(ch)
            })
            .collect()
    }

    /// Return `names` lowercased, ready to hand directly to `filter_reason`.
    pub fn names_lowercased(&self) -> Vec<String> {
        self.names.iter().map(|s| s.to_lowercase()).collect()
    }

    /// Return `include_names` lowercased, ready to hand directly to
    /// `filter_reason`.
    pub fn include_names_lowercased(&self) -> Vec<String> {
        self.include_names
            .iter()
            .map(|s| s.to_lowercase())
            .collect()
    }
}
