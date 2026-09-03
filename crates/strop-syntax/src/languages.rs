//! The curated language registry (0002 §2.2: statically linked, always).
//! All highlight queries are Helix-vendored (queries/ per language, MPL-2.0) —
//! one upstream, one review surface, divergence watched weekly (0002 §7).
//! Adding a language is one row here plus one crate in Cargo.toml — never a
//! download, never a dlopen. C++ included specifically: its scanner is C++,
//! the musl static-libstdc++ path the release gate guards (0002 §5).
//!
//! Detection for a file path goes: exact basename → extension → shebang
//! (first line, only consulted when the extension is unknown or absent).
//! `detect` is pure over `(path, first_line)` so callers own the one
//! cheap read; `Highlighter::for_path` does exactly that.

use std::path::Path;

use tree_sitter::Language;

pub struct LanguageSpec {
    pub name: &'static str,
    pub language: Language,
    pub highlights: &'static str,
}

// tree-sitter-toml 0.20 targets an old ABI (a second tree-sitter in the
// tree — not worth it); TOML joins when a 0.24-ABI grammar crate exists.
macro_rules! lang_fn {
    ($name:literal, $f:expr, $q:expr) => {
        LanguageSpec {
            name: $name,
            language: $f.into(),
            highlights: $q,
        }
    };
}

/// Extension (with dot) → spec. First match wins.
pub fn for_extension(ext: &str) -> Option<LanguageSpec> {
    Some(match ext {
        ".rs" => lang_fn!(
            "rust",
            tree_sitter_rust::LANGUAGE,
            include_str!("../queries/rust/highlights.scm")
        ),
        ".py" | ".pyi" => {
            lang_fn!(
                "python",
                tree_sitter_python::LANGUAGE,
                include_str!("../queries/python/highlights.scm")
            )
        }
        ".js" | ".jsx" | ".mjs" | ".cjs" => {
            lang_fn!(
                "javascript",
                tree_sitter_javascript::LANGUAGE,
                include_str!("../queries/javascript/highlights.scm")
            )
        }
        ".ts" => lang_fn!(
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            include_str!("../queries/typescript/highlights.scm")
        ),
        ".tsx" => lang_fn!(
            "tsx",
            tree_sitter_typescript::LANGUAGE_TSX,
            include_str!("../queries/tsx/highlights.scm")
        ),
        ".go" => lang_fn!(
            "go",
            tree_sitter_go::LANGUAGE,
            include_str!("../queries/go/highlights.scm")
        ),
        ".c" | ".h" => lang_fn!(
            "c",
            tree_sitter_c::LANGUAGE,
            include_str!("../queries/c/highlights.scm")
        ),
        ".cpp" | ".cc" | ".cxx" | ".hpp" | ".hh" => {
            lang_fn!(
                "cpp",
                tree_sitter_cpp::LANGUAGE,
                include_str!("../queries/cpp/highlights.scm")
            )
        }
        ".json" => lang_fn!(
            "json",
            tree_sitter_json::LANGUAGE,
            include_str!("../queries/json/highlights.scm")
        ),
        ".sh" | ".bash" => lang_fn!(
            "bash",
            tree_sitter_bash::LANGUAGE,
            include_str!("../queries/bash/highlights.scm")
        ),
        ".fish" => lang_fn!(
            "fish",
            tree_sitter_fish::language(),
            include_str!("../queries/fish/highlights.scm")
        ),
        ".lua" => lang_fn!(
            "lua",
            tree_sitter_lua::LANGUAGE,
            include_str!("../queries/lua/highlights.scm")
        ),
        ".sql" => lang_fn!(
            "sql",
            tree_sitter_sequel::LANGUAGE,
            include_str!("../queries/sql/highlights.scm")
        ),
        _ => return None,
    })
}

/// Basenames that imply a language regardless of extension. Checked
/// before the extension so dotfiles (`Path::extension` sees none for
/// `.bashrc`) and Arch build scripts resolve.
fn for_basename(name: &str) -> Option<LanguageSpec> {
    (matches!(name, ".bashrc" | ".bash_profile" | ".profile" | "PKGBUILD")).then(|| {
        lang_fn!(
            "bash",
            tree_sitter_bash::LANGUAGE,
            include_str!("../queries/bash/highlights.scm")
        )
    })
}

/// Interpreter named by a shebang line → spec. Only shells live here;
/// one row per language we actually ship a grammar for.
fn for_interpreter(interp: &str) -> Option<LanguageSpec> {
    Some(match interp {
        "bash" | "sh" | "dash" | "zsh" => lang_fn!(
            "bash",
            tree_sitter_bash::LANGUAGE,
            include_str!("../queries/bash/highlights.scm")
        ),
        "fish" => lang_fn!(
            "fish",
            tree_sitter_fish::language(),
            include_str!("../queries/fish/highlights.scm")
        ),
        _ => return None,
    })
}

/// First shebang token as a bare interpreter name: `#!/bin/bash` →
/// `bash`, `#!/usr/bin/env -S fish -e` → `fish`. Returns `None` for
/// anything that isn't a shebang line.
pub fn interpreter_of(first_line: &str) -> Option<&str> {
    let mut tokens = first_line.strip_prefix("#!")?.split_whitespace();
    let program = tokens.next()?;
    // `env` indirection: the interpreter is the next non-flag word
    // (`-S`/`--split-string` and friends).
    let program = if basename(program) == Some("env") {
        tokens.find(|t| !t.starts_with('-'))?
    } else {
        program
    };
    basename(program)
}

fn basename(program: &str) -> Option<&str> {
    Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
}

/// Shebang line → spec.
pub fn for_shebang(first_line: &str) -> Option<LanguageSpec> {
    for_interpreter(interpreter_of(first_line)?)
}

/// Path → spec, pure in `first_line`: exact basename first, then the
/// extension table, then — only when the extension is unknown or
/// absent — whatever the (already-read) first line shebangs to.
pub fn detect(path: &str, first_line: Option<&str>) -> Option<LanguageSpec> {
    let p = Path::new(path);
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        if let Some(spec) = for_basename(name) {
            return Some(spec);
        }
    }
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy()));
    if let Some(spec) = ext.as_deref().and_then(for_extension) {
        return Some(spec);
    }
    first_line.and_then(for_shebang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_the_curated_set() {
        for ext in [
            ".rs", ".py", ".js", ".ts", ".tsx", ".go", ".c", ".cpp", ".json", ".sh", ".fish",
            ".lua", ".sql",
        ] {
            assert!(for_extension(ext).is_some(), "missing {ext}");
        }
        assert!(for_extension(".xyz").is_none());
    }

    #[test]
    fn exact_filenames_beat_extension_and_shebang() {
        for name in [".bashrc", ".bash_profile", ".profile", "PKGBUILD"] {
            let spec = detect(&format!("/home/tarek/{name}"), None)
                .unwrap_or_else(|| panic!("{name} unresolved"));
            assert_eq!(spec.name, "bash", "{name}");
        }
        // exact basename wins even against a contradictory shebang
        let spec = detect("/home/tarek/.bashrc", Some("#!/usr/bin/env fish\n")).unwrap();
        assert_eq!(spec.name, "bash");
        // "PKGBUILD.fish" is not an exact basename — extension rules
        assert_eq!(detect("PKGBUILD.fish", None).unwrap().name, "fish");
    }

    #[test]
    fn shebang_resolves_when_extension_unknown_or_absent() {
        for (line, lang) in [
            ("#!/bin/bash\n", "bash"),
            ("#!/bin/bash -euo pipefail\n", "bash"),
            ("#!/usr/bin/env bash\n", "bash"),
            ("#!/usr/bin/env -S bash --norc\n", "bash"),
            ("#!/bin/sh\n", "bash"),
            ("#!/usr/bin/env zsh\n", "bash"),
            ("#!/usr/bin/fish\n", "fish"),
            ("#!/usr/bin/env fish\n", "fish"),
        ] {
            let spec = detect("some-script", Some(line))
                .unwrap_or_else(|| panic!("unresolved shebang {line:?}"));
            assert_eq!(spec.name, lang, "{line:?}");
        }
        // unknown extension still defers to the shebang
        assert_eq!(
            detect("weird.tool", Some("#!/bin/bash\n")).unwrap().name,
            "bash"
        );
        // no shebang, no extension, no dice
        assert!(detect("README", Some("# comment\n")).is_none());
        assert!(detect("run.pl", Some("#!/usr/bin/perl\n")).is_none());
        assert!(detect("empty", Some("")).is_none());
    }

    #[test]
    fn known_extension_beats_shebang() {
        let spec = detect("x.fish", Some("#!/bin/bash\n")).unwrap();
        assert_eq!(spec.name, "fish");
    }
}
