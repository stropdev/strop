//! The server registry (0009 §2.3): curated, embedded, no auto-download.
//! Missing servers produce a hint, never a fetch. languages.toml layers
//! (0012) override/extend the embedded table before resolution: project
//! `.strop/languages.toml` > XDG `~/.config/strop/languages.toml` >
//! embedded.

use std::path::Path;
use std::sync::LazyLock;

use crate::languages::Languages;

/// How to spawn one language server — borrowed from the registry tables
/// (embedded or merged languages.toml), attach-time only. `name` from a
/// process-lifetime config borrow is `&'static str` in practice.
#[derive(Debug, Clone)]
pub struct ServerSpec<'a> {
    pub name: &'a str,
    pub command: &'a str,
    pub args: &'a [String],
    /// Install hint for registry-known servers; config-defined servers
    /// have none.
    pub install_hint: Option<&'a str>,
    /// initializationOptions — helix's `[language-server.NAME.config]`.
    pub init_options: Option<&'a serde_json::Value>,
}

impl ServerSpec<'_> {
    /// A config-provided command may be absolute (0012 §5) — the PATH
    /// probe is skipped then; the spawn itself checks existence.
    pub fn absolute_command(&self) -> bool {
        Path::new(self.command).is_absolute()
    }
}

struct EmbeddedServer {
    name: &'static str,
    command: &'static str,
    args: Vec<String>,
    hint: &'static str,
}

fn embedded_spec(e: &'static EmbeddedServer) -> ServerSpec<'static> {
    ServerSpec {
        name: e.name,
        command: e.command,
        args: &e.args,
        install_hint: Some(e.hint),
        init_options: None,
    }
}

/// The curated table (0009 §2.3), built once on first use — `ServerSpec`
/// borrows from it, so entries own their strings.
static EMBEDDED: LazyLock<Vec<EmbeddedServer>> = LazyLock::new(|| {
    vec![
        EmbeddedServer {
            name: "rust-analyzer",
            command: "rust-analyzer",
            args: vec![],
            hint: "rustup component add rust-analyzer",
        },
        EmbeddedServer {
            name: "clangd",
            command: "clangd",
            args: vec![],
            hint: "install clangd (brew install llvm / apt install clangd)",
        },
        EmbeddedServer {
            name: "pyright",
            command: "pyright-langserver",
            args: vec!["--stdio".into()],
            hint: "npm i -g pyright",
        },
        EmbeddedServer {
            name: "gopls",
            command: "gopls",
            args: vec![],
            hint: "go install golang.org/x/tools/gopls@latest",
        },
        EmbeddedServer {
            name: "typescript-language-server",
            command: "typescript-language-server",
            args: vec!["--stdio".into()],
            hint: "npm i -g typescript-language-server typescript",
        },
        EmbeddedServer {
            name: "vscode-json-language-server",
            command: "vscode-json-language-server",
            args: vec!["--stdio".into()],
            hint: "npm i -g vscode-langservers-extracted",
        },
        EmbeddedServer {
            name: "bash-language-server",
            command: "bash-language-server",
            args: vec!["start".into()],
            hint: "npm i -g bash-language-server",
        },
    ]
});

fn embedded_by_name(name: &str) -> Option<&'static EmbeddedServer> {
    EMBEDDED.iter().find(|e| e.name == name)
}

pub(crate) fn is_embedded(name: &str) -> bool {
    embedded_by_name(name).is_some()
}

/// File extension (with dot) → language name — the keys users write in
/// `[language.NAME]` and the registry's language vocabulary.
pub fn language_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        ".rs" => "rust",
        ".c" | ".h" => "c",
        ".cpp" | ".cc" | ".cxx" | ".hpp" | ".hh" => "cpp",
        ".py" | ".pyi" => "python",
        ".go" => "go",
        ".js" | ".jsx" | ".mjs" | ".cjs" => "javascript",
        ".ts" | ".tsx" => "typescript",
        ".json" => "json",
        ".sh" | ".bash" => "shellscript",
        _ => return None,
    })
}

fn embedded_name_for_language(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "rust" => "rust-analyzer",
        "c" | "cpp" => "clangd",
        "python" => "pyright",
        "go" => "gopls",
        "javascript" | "typescript" => "typescript-language-server",
        "json" => "vscode-json-language-server",
        "shellscript" => "bash-language-server",
        _ => return None,
    })
}

/// Resolve the server for a file extension through the config layers
/// (0012: project > XDG > embedded). The first resolvable entry of a
/// language's `language-servers` override wins — strop runs one server
/// per workspace root (0009 wave model); an override that resolves to
/// nothing falls back to the embedded default (the layer load already
/// warned about unknown names).
pub fn for_extension<'a>(ext: &str, cfg: &'a Languages) -> Option<ServerSpec<'a>> {
    let lang = language_for_extension(ext)?;
    if let Some(names) = cfg
        .languages
        .get(lang)
        .map(|l| l.language_servers.as_slice())
    {
        for name in names {
            if let Some(spec) = server_by_name(cfg, name) {
                return Some(spec);
            }
        }
    }
    server_by_name(cfg, embedded_name_for_language(lang)?)
}

fn server_by_name<'a>(cfg: &'a Languages, name: &str) -> Option<ServerSpec<'a>> {
    let emb = embedded_by_name(name);
    let Some((key, def)) = cfg.servers.get_key_value(name) else {
        return emb.map(embedded_spec);
    };
    // the command must come from the def or the embedded spec it refines
    let command = def.command.as_deref().or(emb.map(|e| e.command))?;
    Some(ServerSpec {
        name: key.as_str(),
        command,
        args: def
            .args
            .as_deref()
            .or(emb.map(|e| e.args.as_slice()))
            .unwrap_or(&[]),
        install_hint: emb.map(|e| e.hint),
        init_options: def.config.as_ref(),
    })
}

/// Workspace root for a buffer: the git root, else the file's directory.
pub fn workspace_root(path: &Path, cwd: &Path) -> std::path::PathBuf {
    let mut dir = if path.is_absolute() {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| cwd.to_path_buf())
    } else {
        cwd.to_path_buf()
    };
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if !dir.pop() {
            return cwd.to_path_buf();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_languages_have_servers() {
        let cfg = Languages::default();
        for ext in [".rs", ".cpp", ".py", ".go", ".ts", ".json", ".sh"] {
            assert!(for_extension(ext, &cfg).is_some(), "missing {ext}");
        }
        assert!(for_extension(".xyz", &cfg).is_none());
    }

    #[test]
    fn embedded_specs_carry_their_arguments() {
        let cfg = Languages::default();
        let pyright = for_extension(".py", &cfg).unwrap();
        assert_eq!(pyright.name, "pyright");
        assert_eq!(pyright.command, "pyright-langserver");
        assert_eq!(pyright.args.first().map(String::as_str), Some("--stdio"));
        assert_eq!(pyright.install_hint, Some("npm i -g pyright"));
        assert!(pyright.init_options.is_none());
        // embedded commands are PATH-probed, never absolute
        assert!(!pyright.absolute_command());
    }

    #[test]
    fn root_walks_to_git() {
        let dir = std::env::temp_dir().join("strop-lsp-root");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let root = workspace_root(&dir.join("src/deep/f.rs"), &dir);
        assert_eq!(root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
