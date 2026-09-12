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
    /// The command/args come from a PROJECT languages.toml (0020 §15:
    /// executable content — the trust gate keys on this).
    pub project_executable: bool,
}

impl ServerSpec<'_> {
    /// A config-provided command may be absolute (0012 §5) — the
    /// metadata check stats it directly instead of scanning PATH.
    pub fn absolute_command(&self) -> bool {
        Path::new(self.command).is_absolute()
    }
}

/// Pre-spawn executability of a spec's command, settled by metadata
/// alone (0033 §3): nothing is executed before the trust decision and
/// no orphan probe process can exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    /// Resolves to an executable regular file.
    Executable,
    /// Cannot run; the reason names the fix ("no such file",
    /// "not executable", "not found on PATH").
    Unrunnable(&'static str),
}

/// Check the command the spawn would run: absolute or slash-relative
/// commands stat against `root` (the spawn's working directory); bare
/// names scan `path` with execvp's rules, an empty segment meaning
/// `root`. A missing `path` finds nothing.
pub fn command_status(
    spec: &ServerSpec<'_>,
    root: &Path,
    path: Option<&std::ffi::OsStr>,
) -> CommandStatus {
    let command = spec.command;
    if spec.absolute_command() || command.contains(std::path::is_separator) {
        return file_status(&root.join(command));
    }
    let Some(path) = path else {
        return CommandStatus::Unrunnable("not found on PATH");
    };
    for dir in std::env::split_paths(path) {
        let candidate = if dir.as_os_str().is_empty() {
            root.join(command)
        } else {
            dir.join(command)
        };
        if let CommandStatus::Executable = file_status(&candidate) {
            return CommandStatus::Executable;
        }
    }
    CommandStatus::Unrunnable("not found on PATH")
}

fn file_status(path: &Path) -> CommandStatus {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && executable(&metadata) => CommandStatus::Executable,
        Ok(_) => CommandStatus::Unrunnable("not executable"),
        Err(_) => CommandStatus::Unrunnable("no such file"),
    }
}

/// The exec permission bits execvp would require; non-unix targets have
/// no portable bit, so existence is the strongest metadata check there.
#[cfg(unix)]
fn executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &std::fs::Metadata) -> bool {
    true
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
        project_executable: false,
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
pub fn language_for_extension(ext_with_dot: &str) -> Option<&'static str> {
    strop_core::languages::language_for_extension(ext_with_dot)
}

/// Bare extension (no dot) → language name. Pure table lookup, safe on
/// any keystroke path — the canonical catalog lives in strop-core
/// (0051 §3), shared with the query compiler.
pub fn language_for_extension_name(ext: &str) -> Option<&'static str> {
    strop_core::languages::language_for_extension_name(ext)
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
    let project_executable = cfg.project_root.is_some() && cfg.project_commands.contains(name);
    Some(ServerSpec {
        name: key.as_str(),
        command,
        args: def
            .args
            .as_deref()
            .or(emb.map(|e| e.args.as_slice()))
            .unwrap_or(&[]),
        install_hint: if def.command.is_none() {
            emb.map(|e| e.hint)
        } else {
            None
        },
        init_options: def.config.as_ref(),
        project_executable,
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

    /// 0033 §3: the pre-spawn check inspects metadata only — nothing is
    /// executed, so an untrusted project command is never run before
    /// the trust gate and no probe process is orphaned.
    #[test]
    fn command_status_is_a_pure_metadata_check() {
        let fixture = tempfile::tempdir().unwrap();
        let dir = fixture.path();
        let root = dir.join("root");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&bin).unwrap();

        fn status_for(command: &str, root: &Path, path: Option<&std::ffi::OsStr>) -> CommandStatus {
            let merged = Languages::merge(
                None,
                Some(
                    toml::from_str(&format!(
                        "[language-server.custom]\ncommand = \"{command}\"\n\
                         \n[language.python]\nlanguage-servers = [\"custom\"]\n"
                    ))
                    .unwrap(),
                ),
            );
            let spec = for_extension(".py", &merged).unwrap();
            command_status(&spec, root, path)
        }

        // absolute, missing
        assert_eq!(
            status_for("/nonexistent/custom-lsp", &root, None),
            CommandStatus::Unrunnable("no such file")
        );
        // absolute, present: the exec bit decides (existence on non-unix)
        let tool = dir.join("tool");
        std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                status_for(tool.to_str().unwrap(), &root, None),
                CommandStatus::Unrunnable("not executable")
            );
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            status_for(tool.to_str().unwrap(), &root, None),
            CommandStatus::Executable
        );
        // slash-relative commands resolve against the spawn's root
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let rel = root.join("sub/rel-lsp");
        std::fs::write(&rel, b"#!/bin/sh\n").unwrap();
        make_executable(&rel);
        assert_eq!(
            status_for("sub/rel-lsp", &root, None),
            CommandStatus::Executable
        );
        // bare names scan the provided PATH, never the process's own
        let onpath = bin.join("onpath-lsp");
        std::fs::write(&onpath, b"#!/bin/sh\n").unwrap();
        make_executable(&onpath);
        assert_eq!(
            status_for("onpath-lsp", &root, Some(bin.as_os_str())),
            CommandStatus::Executable
        );
        assert_eq!(
            status_for("nowhere-lsp", &root, Some(bin.as_os_str())),
            CommandStatus::Unrunnable("not found on PATH")
        );
        assert_eq!(
            status_for("onpath-lsp", &root, None),
            CommandStatus::Unrunnable("not found on PATH")
        );
        // execvp treats an empty PATH segment as the current directory
        // — the spawn's working directory, i.e. the root
        let incwd = root.join("incwd-lsp");
        std::fs::write(&incwd, b"#!/bin/sh\n").unwrap();
        make_executable(&incwd);
        assert_eq!(
            status_for("incwd-lsp", &root, Some(std::ffi::OsStr::new(""))),
            CommandStatus::Executable
        );
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

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
