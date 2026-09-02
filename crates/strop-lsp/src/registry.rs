//! The server registry (0009 §2.3): curated, embedded, no auto-download.
//! Missing servers produce a hint, never a fetch.

use std::path::Path;

/// How to spawn one language server.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub name: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    /// Shown once per session when the server isn't on PATH.
    pub install_hint: &'static str,
}

/// File extension (with dot) → server. Config overrides land with 0005.
pub fn for_extension(ext: &str) -> Option<ServerSpec> {
    Some(match ext {
        ".rs" => ServerSpec {
            name: "rust-analyzer",
            command: "rust-analyzer",
            args: &[],
            install_hint: "rustup component add rust-analyzer",
        },
        ".c" | ".h" | ".cpp" | ".cc" | ".cxx" | ".hpp" | ".hh" => ServerSpec {
            name: "clangd",
            command: "clangd",
            args: &[],
            install_hint: "install clangd (brew install llvm / apt install clangd)",
        },
        ".py" | ".pyi" => ServerSpec {
            name: "pyright",
            command: "pyright-langserver",
            args: &["--stdio"],
            install_hint: "npm i -g pyright",
        },
        ".go" => ServerSpec {
            name: "gopls",
            command: "gopls",
            args: &[],
            install_hint: "go install golang.org/x/tools/gopls@latest",
        },
        ".js" | ".jsx" | ".mjs" | ".cjs" | ".ts" | ".tsx" => ServerSpec {
            name: "typescript-language-server",
            command: "typescript-language-server",
            args: &["--stdio"],
            install_hint: "npm i -g typescript-language-server typescript",
        },
        ".json" => ServerSpec {
            name: "vscode-json-language-server",
            command: "vscode-json-language-server",
            args: &["--stdio"],
            install_hint: "npm i -g vscode-langservers-extracted",
        },
        ".sh" | ".bash" => ServerSpec {
            name: "bash-language-server",
            command: "bash-language-server",
            args: &["start"],
            install_hint: "npm i -g bash-language-server",
        },
        _ => return None,
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
    fn curated_extensions_have_servers() {
        for ext in [".rs", ".cpp", ".py", ".go", ".ts", ".json", ".sh"] {
            assert!(for_extension(ext).is_some(), "missing {ext}");
        }
        assert!(for_extension(".xyz").is_none());
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
