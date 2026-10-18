// SPDX-License-Identifier: GPL-3.0-or-later
//! The hand-edited settings file.
//!
//! ```toml
//! # ~/.config/wlrix/power.toml
//! indicator = "auto"           # auto | hidden
//! warnings = true              # notify when the battery runs low
//! ```
//!
//! Read from the user's config directory first, then `/etc/wlrix`; the first file found wins
//! outright, and unknown keys are an error -- the shape every wlRIX component's file has.

use serde::Deserialize;

/// Whether the battery icon is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Indicator {
    /// Shown whenever there is a battery, hidden otherwise -- so a desktop shows nothing.
    #[default]
    Auto,
    /// Never shown. The low-battery warnings still come, unless `warnings = false`: they are the
    /// part a person cannot afford to miss, and hiding an icon is not asking to miss them.
    Hidden,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub indicator: Indicator,
    #[serde(default)]
    warnings: Option<bool>,
}

impl Config {
    /// Whether to notify as the battery runs low. On unless turned off.
    pub fn warnings(&self) -> bool {
        self.warnings.unwrap_or(true)
    }

    /// Load the first config file that exists. No file at all is the defaults.
    pub fn load() -> Self {
        for path in crate::xdg::config_paths() {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    eprintln!("wlrix-power: could not read {}: {err}", path.display());
                    continue;
                }
            };
            match toml::from_str::<Self>(&text) {
                Ok(config) => return config,
                Err(err) => {
                    eprintln!("wlrix-power: {} is not valid: {err}", path.display());
                    return Self::default();
                }
            }
        }
        Self::default()
    }
}

/// Parse a candidate config file, for `--check-config`. `wlrix-settings-daemon` runs this before
/// renaming a file into place, so a settings panel cannot write one this program would refuse.
pub fn check(path: &std::path::Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    toml::from_str::<Config>(&text)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_shows_the_icon_and_warns() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.indicator, Indicator::Auto);
        assert!(config.warnings());
    }

    #[test]
    fn both_keys_can_be_turned_off() {
        let config: Config = toml::from_str("indicator = \"hidden\"\nwarnings = false\n").unwrap();
        assert_eq!(config.indicator, Indicator::Hidden);
        assert!(!config.warnings());
    }

    #[test]
    fn a_typo_is_refused_rather_than_ignored() {
        assert!(toml::from_str::<Config>("indicator = \"off\"\n").is_err());
        assert!(toml::from_str::<Config>("warning = false\n").is_err());
    }
}
