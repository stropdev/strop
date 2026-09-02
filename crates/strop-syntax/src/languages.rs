//! The curated language registry (0002 §2.2: statically linked, always).
//! All highlight queries are Helix-vendored (queries/ per language, MPL-2.0) —
//! one upstream, one review surface, divergence watched weekly (0002 §7).
//! Extension → (grammar, highlights query). Adding a language is one
//! row here plus one crate in Cargo.toml — never a download, never a
//! dlopen. C++ included specifically: its scanner is C++, the musl
//! static-libstdc++ path the release gate guards (0002 §5).

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
        ".c" | ".h" => lang_fn!("c", tree_sitter_c::LANGUAGE, include_str!("../queries/c/highlights.scm")),
        ".cpp" | ".cc" | ".cxx" | ".hpp" | ".hh" => {
            lang_fn!(
                "cpp",
                tree_sitter_cpp::LANGUAGE,
                tree_sitter_cpp::HIGHLIGHT_QUERY
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_the_curated_set() {
        for ext in [
            ".rs", ".py", ".js", ".ts", ".tsx", ".go", ".c", ".cpp", ".json", ".sh",
        ] {
            assert!(for_extension(ext).is_some(), "missing {ext}");
        }
        assert!(for_extension(".xyz").is_none());
    }
}
