//! Configuration (0005-lite): real TOML from day one, embedded defaults,
//! never bricks (0005 §2). The full layering/hot-reload/settings-popup
//! arrives with 0005 proper; this is the editor-facing config object.
//!
//! `$XDG_CONFIG_HOME/strop/config.toml` (or ~/.config/strop/config.toml):
//! ```toml
//! tab_size = 4
//! ```

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Indent unit in spaces (`>>`, auto-indent). Tabs land with 0005's
    /// full option set.
    pub tab_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { tab_size: 4 }
    }
}

impl Config {
    /// Load the user config; errors are returned as a message for the
    /// statusline — the editor always starts with defaults (0005 §2).
    pub fn load() -> (Self, Option<String>) {
        let Some(path) = config_path() else {
            return (Self::default(), None);
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (Self::default(), None); // absent is fine
        };
        match toml::from_str::<Config>(&text) {
            Ok(c) => (c, None),
            Err(e) => (
                Self::default(),
                Some(format!("config {}: {e} — using defaults", path.display())),
            ),
        }
    }

    pub fn indent(&self) -> String {
        " ".repeat(self.tab_size)
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("strop").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_absent() {
        let (c, err) = Config::load();
        let _ = err; // present only when a malformed file exists
        assert!(c.tab_size >= 2);
    }

    #[test]
    fn parses_tab_size() {
        let c: Config = toml::from_str("tab_size = 2").unwrap();
        assert_eq!(c.tab_size, 2);
        assert_eq!(c.indent(), "  ");
    }

    #[test]
    fn malformed_falls_back() {
        assert!(toml::from_str::<Config>("tab_size = \"oops\"").is_err());
    }
}
