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

/// Supported indent-width range (0051 R08): manual `:tab-size`
/// overrides AND the `tab_size` config value. Zero would make Tab a
/// no-op with a hangover of stale guides; huge values are allocation
/// and layout hazards — both are refused visibly, never clamped
/// silently.
pub const TAB_SIZE_MIN: usize = 1;
pub const TAB_SIZE_MAX: usize = 16;

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
    /// Search surfaces show unignored dotfiles/dotfolders by default
    /// (0051 R03). `hidden:include|exclude` in a query overrides.
    pub search_show_hidden: bool,
    /// Search surfaces respect .gitignore/.ignore/.rgignore (0051 R03).
    /// `ignored:include|exclude` in a query overrides.
    pub search_respect_ignore: bool,
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
            search_show_hidden: true,
            search_respect_ignore: true,
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
    Knob {
        key: "search_show_hidden",
        kind: "bool",
        desc: "search shows dotfiles by default",
    },
    Knob {
        key: "search_respect_ignore",
        kind: "bool",
        desc: "search respects ignore files",
    },
];

impl Config {
    /// The one typed knob→value projection (0051 R10): `print_knobs`,
    /// `:explain` and selectors read real values from this path. A key
    /// absent from KNOBS is `None` — never a fabricated "?" placeholder.
    pub fn knob_value(&self, key: &str) -> Option<String> {
        Some(match key {
            "tab_size" => self.tab_size.to_string(),
            "indent_guides" => self.indent_guides.to_string(),
            "indent_style" => format!("{:?}", self.indent_style).to_lowercase(),
            "indent_detect" => self.indent_detect.to_string(),
            "auto_format" => self.auto_format.to_string(),
            "search_show_hidden" => self.search_show_hidden.to_string(),
            "search_respect_ignore" => self.search_respect_ignore.to_string(),
            _ => return None,
        })
    }

    /// `strop config`: the knobs with live values (KNOBS is the data
    /// source; this is its first consumer — the settings popup is next).
    pub fn print_knobs(&self) {
        for k in KNOBS {
            let Some(value) = self.knob_value(k.key) else {
                continue; // tests pin every KNOBS key to a value
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
        let parsed = toml::from_str::<Config>(&text)
            .map_err(|e| e.to_string())
            .and_then(Config::validated);
        match parsed {
            Ok(c) => (c, None),
            Err(e) => (
                Self::default(),
                Some(format!("config {}: {e} — using defaults", path.display())),
            ),
        }
    }

    /// `tab_size` outside the supported range (0051 R08): refused
    /// visibly like any malformed config — never a silent clamp, never
    /// a zero-width Tab or an enormous indent allocation.
    fn validated(self) -> Result<Self, String> {
        if (TAB_SIZE_MIN..=TAB_SIZE_MAX).contains(&self.tab_size) {
            Ok(self)
        } else {
            Err(format!(
                "tab_size must be {TAB_SIZE_MIN}–{TAB_SIZE_MAX}, got {}",
                self.tab_size
            ))
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
            7,
            "tab_size, indent_guides, indent_style, indent_detect, auto_format, search_show_hidden, search_respect_ignore"
        );
    }

    #[test]
    fn every_knob_resolves_a_real_value() {
        // 0051 R10: one typed access path; a knob that resolves to
        // None would render as a placeholder or vanish from :explain.
        let config = Config::default();
        for knob in KNOBS {
            let value = config.knob_value(knob.key);
            assert!(value.is_some(), "knob {:?} has no value", knob.key);
            assert_ne!(
                value.as_deref(),
                Some("?"),
                "knob {:?} is a placeholder",
                knob.key
            );
        }
        assert!(config.knob_value("not_a_knob").is_none());
    }

    #[test]
    fn malformed_falls_back() {
        assert!(toml::from_str::<Config>("tab_size = \"oops\"").is_err());
    }
}
