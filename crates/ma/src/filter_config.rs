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
//! # Merge halfwidth katakana into fullwidth standard forms
//! merge_halfwidth: true
//! ```
//!
//! Both keys are optional and default to empty lists. Any additional chars or
//! names supplied via CLI `--filter` / `--filter-name` are **appended** to the
//! values loaded from the file so the two sources compose rather than compete.

use std::path::Path;

use serde::Deserialize;

/// Filter configuration loaded from a YAML file.
///
/// Both fields are optional in the file; absent keys deserialise to an empty
/// `Vec`.
#[derive(Debug, Default, Deserialize)]
pub struct FilterConfig {
    /// Individual Unicode characters to exclude.
    #[serde(default)]
    pub chars: Vec<String>,

    /// Unicode-name substrings to exclude (matched case-insensitively).
    #[serde(default)]
    pub names: Vec<String>,

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
        self.chars
            .iter()
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

    /// Return names lowercased, ready to hand directly to `filter_reason`.
    pub fn names_lowercased(&self) -> Vec<String> {
        self.names.iter().map(|s| s.to_lowercase()).collect()
    }
}
