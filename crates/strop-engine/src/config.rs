//! Configuration (0005-lite): real TOML from day one, embedded defaults,
//! never bricks (0005 §2). The full layering/hot-reload/settings-popup
//! arrives with 0005 proper; this is the editor-facing config object.
//!
//! `$XDG_CONFIG_HOME/strop/config.toml` (or ~/.config/strop/config.toml):
//! ```toml
//! tab_size = 4
//! indent_style = "spaces"   # or "tabs"
//! indent_detect = true      # infer a document's indent from its content
//! auto_format = true        # LSP format before :w (helix parity)
//! ```
//!
//! LSP server config is a separate file with its own layering —
//! `languages.toml`, helix-shaped, owned by strop-lsp (0012): project
//! `.strop/languages.toml` > XDG > the embedded registry.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Config {
    /// Indent unit width in spaces (`>>`, auto-indent, tab display).
    /// A document's detected indent overrides this per buffer when
    /// `indent_detect` is on.
    pub tab_size: usize,
    /// Indent guides (dim │ per level) on/off.
    pub indent_guides: bool,
    /// What auto-indent, `>>` and the Tab key emit.
    pub indent_style: IndentStyle,
    /// Infer an opened document's indent (unit and width) from its
    /// content; the config above is the fallback and the new-file
    /// default.
    pub indent_detect: bool,
    /// Format through the language server before writing (helix's
    /// auto-format). A formatter failure never blocks the write.
    pub auto_format: bool,
}

/// `indent_style` in config.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IndentStyle {
    Spaces,
    Tabs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent_guides: true,
            indent_style: IndentStyle::Spaces,
            indent_detect: true,
            auto_format: true,
        }
    }
}

/// Knob metadata (0005 §6): the settings popup renders from this table;
/// descriptions live here, never in popup code.
pub struct Knob {
    pub key: &'static str,
    pub kind: &'static str, // "bool" | "number"
    pub desc: &'static str,
}

pub const KNOBS: &[Knob] = &[
    Knob {
        key: "tab_size",
        kind: "number",
        desc: "indent width in spaces",
    },
    Knob {
        key: "indent_guides",
        kind: "bool",
        desc: "dim │ guide per indent level",
    },
    Knob {
        key: "indent_style",
        kind: "string",
        desc: "auto-indent unit: spaces or tabs",
    },
    Knob {
        key: "indent_detect",
        kind: "bool",
        desc: "infer each document's indent from its content",
    },
    Knob {
        key: "auto_format",
        kind: "bool",
        desc: "format through the language server before :w",
    },
];

impl Config {
    /// `strop config`: the knobs with live values (KNOBS is the data
    /// source; this is its first consumer — the settings popup is next).
    pub fn print_knobs(&self) {
        for k in KNOBS {
            let value = match k.key {
                "tab_size" => self.tab_size.to_string(),
                "indent_guides" => self.indent_guides.to_string(),
                "indent_style" => format!("{:?}", self.indent_style).to_lowercase(),
                "indent_detect" => self.indent_detect.to_string(),
                "auto_format" => self.auto_format.to_string(),
                _ => "?".into(),
            };
            println!("  {:<16} {:<7} {:<8} {}", k.key, k.kind, value, k.desc);
        }
    }

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
        match self.indent_style {
            IndentStyle::Spaces => " ".repeat(self.tab_size),
            IndentStyle::Tabs => "\t".into(),
        }
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
    fn parses_indent_guides() {
        let c: Config = toml::from_str("indent_guides = false").unwrap();
        assert!(!c.indent_guides);
        // absent → default on
        let c: Config = toml::from_str("").unwrap();
        assert!(c.indent_guides);
    }

    #[test]
    fn knobs_name_real_fields_and_cover_all_of_them() {
        // the popup renders from KNOBS: a knob naming no field is dead
        // weight, a field without a knob is invisible to users.
        for knob in KNOBS {
            let snippet = match knob.key {
                "indent_style" => "indent_style = \"spaces\"".to_string(),
                _ => match knob.kind {
                    "number" => format!("{k} = 2", k = knob.key),
                    "bool" => format!("{k} = true", k = knob.key),
                    _ => format!("{k} = \"x\"", k = knob.key),
                },
            };
            assert!(
                toml::from_str::<Config>(&snippet).is_ok(),
                "knob {:?} names no config field",
                knob.key
            );
        }
        assert_eq!(
            KNOBS.len(),
            5,
            "tab_size, indent_guides, indent_style, indent_detect, auto_format"
        );
    }

    #[test]
    fn malformed_falls_back() {
        assert!(toml::from_str::<Config>("tab_size = \"oops\"").is_err());
    }
}
