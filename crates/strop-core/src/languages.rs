//! The canonical language metadata catalog (0051 §3): ONE in-repo table
//! mapping file extensions to language names and back. The LSP registry
//! and the query compiler both consume this — there is no second table.

/// One language family: canonical name, filename extensions, documented
/// value aliases (`rs` reads as `rust`).
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub aliases: &'static [&'static str],
}

pub const LANGUAGES: &[Language] = &[
    Language {
        name: "rust",
        extensions: &["rs"],
        aliases: &["rs"],
    },
    Language {
        name: "c",
        extensions: &["c", "h"],
        aliases: &[],
    },
    Language {
        name: "cpp",
        extensions: &["cpp", "cc", "cp", "cxx", "hpp", "hh", "hxx", "ino", "tpp"],
        aliases: &["c++", "cxx"],
    },
    Language {
        name: "lua",
        extensions: &["lua"],
        aliases: &["lua"],
    },
    Language {
        name: "python",
        extensions: &["py", "pyi", "pyw"],
        aliases: &["py"],
    },
    Language {
        name: "go",
        extensions: &["go"],
        aliases: &["golang"],
    },
    Language {
        name: "javascript",
        extensions: &["js", "jsx", "mjs", "cjs"],
        aliases: &["js"],
    },
    Language {
        name: "typescript",
        extensions: &["ts", "tsx", "mts", "cts"],
        aliases: &["ts"],
    },
    Language {
        name: "json",
        extensions: &["json", "jsonc"],
        aliases: &[],
    },
    Language {
        name: "shellscript",
        extensions: &["sh", "bash", "zsh", "ksh"],
        aliases: &["sh", "bash", "shell"],
    },
];

/// Language name for a bare extension (no dot).
pub fn language_for_extension_name(ext: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|language| language.extensions.contains(&ext))
        .map(|language| language.name)
}

/// Language name for a dotted extension (`.rs`).
pub fn language_for_extension(ext_with_dot: &str) -> Option<&'static str> {
    language_for_extension_name(ext_with_dot.strip_prefix('.')?)
}

/// A language's extensions, by canonical name or documented alias.
pub fn extensions_for_language(name_or_alias: &str) -> Option<&'static [&'static str]> {
    LANGUAGES
        .iter()
        .find(|language| {
            language.name == name_or_alias || language.aliases.contains(&name_or_alias)
        })
        .map(|language| language.extensions)
}

/// Every canonical language name (suggestions/discovery).
pub fn language_names() -> impl Iterator<Item = &'static str> {
    LANGUAGES.iter().map(|language| language.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_resolves_through_extension_name_and_alias() {
        assert_eq!(language_for_extension(".lua"), Some("lua"));
        assert_eq!(language_for_extension_name("lua"), Some("lua"));
        assert_eq!(extensions_for_language("lua"), Some(&["lua"][..]));
        assert!(language_names().any(|name| name == "lua"));
    }

    #[test]
    fn ambiguous_headers_follow_one_policy() {
        // One policy for ambiguous headers (0063 §3): plain .h is C;
        // C++-spelled headers are cpp.
        assert_eq!(language_for_extension(".h"), Some("c"));
        for header in [".hpp", ".hh", ".hxx"] {
            assert_eq!(language_for_extension(header), Some("cpp"));
        }
    }

    #[test]
    fn cpp_membership_covers_the_documented_extensions() {
        assert_eq!(
            extensions_for_language("c++"),
            Some(&["cpp", "cc", "cp", "cxx", "hpp", "hh", "hxx", "ino", "tpp",][..])
        );
    }
}
