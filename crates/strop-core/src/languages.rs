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
        extensions: &["cpp", "cc", "cxx", "hpp", "hh"],
        aliases: &["c++"],
    },
    Language {
        name: "python",
        extensions: &["py", "pyi"],
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
        extensions: &["ts", "tsx"],
        aliases: &["ts"],
    },
    Language {
        name: "json",
        extensions: &["json"],
        aliases: &[],
    },
    Language {
        name: "shellscript",
        extensions: &["sh", "bash"],
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
