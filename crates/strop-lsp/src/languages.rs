//! languages.toml layers (0012): helix-style server config, project over
//! XDG over the embedded registry. Loading is attach-time only — never
//! the input path — and a broken layer warns instead of bricking
//! (0005 §2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One `languages.toml` layer, helix-shaped (0012 §2): named server
/// definitions plus per-language server overrides. Unknown keys are
/// ignored — users paste helix configs carrying `scope`/`roots`/`
/// `file-types`, and compat beats 0005's reject-unknown strictness here.
/// Helix's `[[language]]` array-of-tables form is *not* accepted;
/// flatten pasted headers to `[language.NAME]` maps.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguagesToml {
    /// `[language-server.NAME]`: command / args / config.
    #[serde(rename = "language-server")]
    pub language_server: BTreeMap<String, ServerDef>,
    /// `[language.LANG]`: `language-servers = [...]`.
    pub language: BTreeMap<String, LanguageDef>,
}

/// A server definition. Every field is optional so an entry naming a
/// registry server can *refine* it — set only what changes, the rest
/// inherits from the embedded spec.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerDef {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    /// `[language-server.NAME.config]` — arbitrary table, passed through
    /// as initializationOptions.
    pub config: Option<serde_json::Value>,
}

/// `[language.LANG]`: which servers the language uses.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguageDef {
    #[serde(rename = "language-servers")]
    pub language_servers: Vec<String>,
}

/// The merged layers (0012 §1: project > XDG > embedded) plus where the
/// project layer came from — the registry resolves through this.
#[derive(Debug, Default)]
pub struct Languages {
    pub servers: BTreeMap<String, ServerDef>,
    pub languages: BTreeMap<String, LanguageDef>,
    /// Directory holding the project layer's `.strop/languages.toml`,
    /// when one was found — it anchors the workspace root (0012 §6).
    pub project_root: Option<PathBuf>,
    warnings: Vec<String>,
}

impl Languages {
    /// Layer problems (parse failures, unspawnable defs, unknown server
    /// names) — surfaced at attach; never fatal (0005 §2).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Read and merge the layers that exist. Absent files are fine;
    /// unreadable or unparseable layers are rejected with a warning and
    /// the remaining layers carry on.
    pub fn load(xdg: Option<&Path>, project: Option<&Path>) -> Self {
        let mut warnings = Vec::new();
        let xdg_file = xdg.and_then(|p| read_layer(p, &mut warnings));
        let project_file = project.and_then(|p| read_layer(p, &mut warnings));
        let mut merged = Self::merge(xdg_file, project_file);
        // `.strop/languages.toml` — the project root is the dir holding
        // `.strop`, two parents up from the file
        merged.project_root = project
            .and_then(|p| p.parent().and_then(Path::parent))
            .map(Path::to_path_buf);
        warnings.append(&mut merged.warnings);
        merged.warnings = warnings;
        merged
    }

    /// Layer merge: per key, the project entry replaces the XDG entry —
    /// everything XDG configured for other keys survives.
    fn merge(xdg: Option<LanguagesToml>, project: Option<LanguagesToml>) -> Self {
        let mut servers = BTreeMap::new();
        let mut languages = BTreeMap::new();
        for layer in [xdg, project].into_iter().flatten() {
            servers.extend(layer.language_server);
            languages.extend(layer.language);
        }
        let mut warnings = Vec::new();
        // a def with no command can only refine a registry server; a
        // command-less def for an unknown name can never spawn
        let unspawnable: Vec<String> = servers
            .iter()
            .filter(|(name, def)| def.command.is_none() && !crate::registry::is_embedded(name))
            .map(|(name, _)| name.clone())
            .collect();
        for name in &unspawnable {
            servers.remove(name);
            warnings.push(format!("language-server.{name}: no command — ignored"));
        }
        // typo guard: names that resolve to nothing (config nor registry)
        for (lang, def) in &languages {
            for name in &def.language_servers {
                if !servers.contains_key(name) && !crate::registry::is_embedded(name) {
                    warnings.push(format!("language.{lang}: unknown server {name}"));
                }
            }
        }
        Self {
            servers,
            languages,
            project_root: None,
            warnings,
        }
    }
}

/// `$XDG_CONFIG_HOME/strop/languages.toml` (or ~/.config/strop/…).
pub fn xdg_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("strop").join("languages.toml"))
}

/// Nearest ancestor of `buffer` holding `.strop/languages.toml` — the
/// project layer (0012 §1). None when there is no project config.
pub fn project_path(buffer: &Path) -> Option<PathBuf> {
    let mut dir = buffer.parent()?.to_path_buf();
    loop {
        let f = dir.join(".strop").join("languages.toml");
        if f.is_file() {
            return Some(f);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn read_layer(path: &Path, warnings: &mut Vec<String>) -> Option<LanguagesToml> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warnings.push(format!("{}: {e} — layer ignored", path.display()));
            return None;
        }
    };
    match toml::from_str::<LanguagesToml>(&text) {
        Ok(f) => Some(f),
        Err(e) => {
            warnings.push(format!("{}: {e} — layer ignored", path.display()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    /// The helix-book pyright pattern: an extraPaths config table on a
    /// registry server, a config-defined server, and a language override.
    const HELIX_STYLE: &str = r#"
[language-server.mypy-lsp]
command = "mypy-langserver"
args = ["--stdio"]

[language-server.pyright.config.python.analysis]
extraPaths = ["lib", "tools"]

[language.python]
language-servers = ["pyright", "mypy-lsp"]
"#;

    fn file(text: &str) -> LanguagesToml {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn parses_helix_style_fixture() {
        let f = file(HELIX_STYLE);
        let pyright = f.language_server.get("pyright").unwrap();
        assert_eq!(pyright.command, None); // inherits the embedded command
        let extra = &pyright.config.as_ref().unwrap()["python"]["analysis"]["extraPaths"];
        assert_eq!(extra, &serde_json::json!(["lib", "tools"]));
        let mypy = f.language_server.get("mypy-lsp").unwrap();
        assert_eq!(mypy.command.as_deref(), Some("mypy-langserver"));
        assert_eq!(
            mypy.args.as_deref(),
            Some(["--stdio".to_string()].as_slice())
        );
        assert_eq!(
            f.language["python"].language_servers,
            ["pyright", "mypy-lsp"]
        );
    }

    #[test]
    fn resolution_project_over_xdg_over_embedded() {
        let xdg = file(
            r#"
[language-server.pyright.config.python.analysis]
extraPaths = ["xdg"]
"#,
        );
        let project = file(
            r#"
[language-server.pyright.config.python.analysis]
extraPaths = ["project"]
"#,
        );
        let merged = Languages::merge(Some(xdg), Some(project));

        // project def replaces the XDG def for the same name; command
        // and args inherit from the embedded spec
        let spec = registry::for_extension(".py", &merged).unwrap();
        let extra = &spec.init_options.unwrap()["python"]["analysis"]["extraPaths"];
        assert_eq!(extra, &serde_json::json!(["project"]));
        assert_eq!(spec.command, "pyright-langserver");
        assert_eq!(spec.args.first().map(String::as_str), Some("--stdio"));

        // embedded fallback: untouched languages resolve unmodified
        let go = registry::for_extension(".go", &merged).unwrap();
        assert_eq!(go.name, "gopls");
        assert!(go.init_options.is_none());

        // XDG alone (no project layer) still overrides
        let only_xdg = Languages::merge(
            Some(file(
                r#"
[language-server.pyright.config.python.analysis]
extraPaths = ["xdg"]
"#,
            )),
            None,
        );
        let spec = registry::for_extension(".py", &only_xdg).unwrap();
        let extra = &spec.init_options.unwrap()["python"]["analysis"]["extraPaths"];
        assert_eq!(extra, &serde_json::json!(["xdg"]));
    }

    #[test]
    fn language_override_picks_the_config_defined_server() {
        let merged = Languages::merge(None, Some(file(HELIX_STYLE)));
        // first resolvable list entry wins (one server per workspace)
        let spec = registry::for_extension(".py", &merged).unwrap();
        assert_eq!(spec.name, "pyright");
        assert_eq!(spec.install_hint, Some("npm i -g pyright"));

        let swapped = Languages::merge(
            None,
            Some(file(
                r#"
[language-server.mypy-lsp]
command = "mypy-langserver"
args = ["--stdio"]

[language.python]
language-servers = ["mypy-lsp"]
"#,
            )),
        );
        let spec = registry::for_extension(".py", &swapped).unwrap();
        assert_eq!(spec.command, "mypy-langserver");
        assert_eq!(spec.install_hint, None); // config-defined, no hint
        assert!(spec.init_options.is_none());
    }

    #[test]
    fn absolute_commands_skip_the_path_probe() {
        let merged = Languages::merge(
            None,
            Some(file(
                r#"
[language-server.custom]
command = "/opt/lsp/custom-lsp"

[language.python]
language-servers = ["custom"]
"#,
            )),
        );
        let spec = registry::for_extension(".py", &merged).unwrap();
        assert!(spec.absolute_command());
        // embedded commands are bare names — probed on PATH as before
        assert!(!registry::for_extension(".rs", &merged)
            .unwrap()
            .absolute_command());
    }

    #[test]
    fn per_language_granularity_of_the_merge() {
        let merged = Languages::merge(
            Some(file(
                r#"
[language.rust]
language-servers = ["rust-analyzer"]
"#,
            )),
            Some(file(
                r#"
[language.python]
language-servers = ["pyright"]
"#,
            )),
        );
        // XDG's rust entry survives a project layer that only touches
        // python; both resolve to their embedded servers
        assert_eq!(
            registry::for_extension(".rs", &merged).unwrap().name,
            "rust-analyzer"
        );
        assert_eq!(
            registry::for_extension(".py", &merged).unwrap().name,
            "pyright"
        );
    }

    #[test]
    fn project_root_is_the_project_layers_dir() {
        let dir = std::env::temp_dir().join("strop-lsp-languages");
        let _ = std::fs::remove_dir_all(&dir);
        let layer = dir.join(".strop");
        std::fs::create_dir_all(&layer).unwrap();
        std::fs::write(layer.join("languages.toml"), HELIX_STYLE).unwrap();

        let loaded = Languages::load(None, Some(&layer.join("languages.toml")));
        assert_eq!(loaded.project_root.as_deref(), Some(dir.as_path()));
        assert!(loaded.warnings().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_layer_warns_and_never_bricks() {
        let dir = std::env::temp_dir().join("strop-lsp-languages-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("languages.toml");
        std::fs::write(&bad, "language-server = 3").unwrap(); // wrong type

        let loaded = Languages::load(Some(&bad), None);
        assert!(!loaded.warnings().is_empty());
        // the embedded registry still resolves
        assert!(registry::for_extension(".rs", &loaded).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unspawnable_and_unknown_names_warn() {
        let merged = Languages::merge(
            None,
            Some(file(
                r#"
[language-server.ghost]
args = ["x"]

[language.python]
language-servers = ["nope"]
"#,
            )),
        );
        assert!(merged.warnings().iter().any(|w| w.contains("ghost")));
        assert!(merged.warnings().iter().any(|w| w.contains("nope")));
        // an override that resolves to nothing falls back to the
        // embedded server — losing LSP over a typo would be worse
        let spec = registry::for_extension(".py", &merged).unwrap();
        assert_eq!(spec.name, "pyright");
    }

    #[test]
    fn helix_extra_keys_are_ignored() {
        // a pasted helix entry carrying keys we don't speak must still
        // parse and apply the parts we do (0012 §3)
        let merged = Languages::merge(
            None,
            Some(file(
                r#"
[language-server.pyright]
scope = "source.python"

[language.python]
file-types = ["py"]
roots = ["pyproject.toml"]
language-servers = ["pyright"]
"#,
            )),
        );
        assert!(merged.warnings().is_empty());
        assert_eq!(
            registry::for_extension(".py", &merged).unwrap().name,
            "pyright"
        );
    }

    #[test]
    fn project_layer_discovery_walks_up() {
        let dir = std::env::temp_dir().join("strop-lsp-languages-walk");
        let _ = std::fs::remove_dir_all(&dir);
        let layer = dir.join(".strop");
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(&layer).unwrap();
        std::fs::write(layer.join("languages.toml"), "").unwrap();

        let found = project_path(&dir.join("src/deep/main.py")).unwrap();
        assert_eq!(found, layer.join("languages.toml"));
        assert!(project_path(&std::env::temp_dir().join("nowhere.rs")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
