//! The curated language registry (0002 §2.2: statically linked, always).
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
            tree_sitter_rust::HIGHLIGHTS_QUERY
        ),
        ".py" | ".pyi" => {
            lang_fn!(
                "python",
                tree_sitter_python::LANGUAGE,
                tree_sitter_python::HIGHLIGHTS_QUERY
            )
        }
        ".js" | ".jsx" | ".mjs" | ".cjs" => {
            lang_fn!(
                "javascript",
                tree_sitter_javascript::LANGUAGE,
                tree_sitter_javascript::HIGHLIGHT_QUERY
            )
        }
        ".ts" => lang_fn!(
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        ".tsx" => lang_fn!(
            "tsx",
            tree_sitter_typescript::LANGUAGE_TSX,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        ".go" => lang_fn!(
            "go",
            tree_sitter_go::LANGUAGE,
            tree_sitter_go::HIGHLIGHTS_QUERY
        ),
        ".c" | ".h" => lang_fn!("c", tree_sitter_c::LANGUAGE, tree_sitter_c::HIGHLIGHT_QUERY),
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
            tree_sitter_json::HIGHLIGHTS_QUERY
        ),
        ".sh" | ".bash" => lang_fn!(
            "bash",
            tree_sitter_bash::LANGUAGE,
            tree_sitter_bash::HIGHLIGHT_QUERY
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
