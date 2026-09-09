//! One statically linked registry for native filenames, shebangs and injections.
//! Queries are vendored from Helix under MPL-2.0; no runtime grammar downloads.
use std::path::Path;
use tree_sitter::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    C,
    Cpp,
    Json,
    Bash,
    Fish,
    Lua,
    Sql,
    CMake,
    Markdown,
    MarkdownInline,
    Java,
    CSharp,
    Ruby,
    Php,
    Toml,
    Yaml,
    Html,
    Css,
}

pub struct LanguageSpec {
    pub id: LanguageId,
    pub name: &'static str,
    pub language: Language,
    pub highlights: &'static str,
    pub injections: &'static str,
}
struct Entry {
    id: LanguageId,
    name: &'static str,
    grammar: fn() -> Language,
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
    interpreters: &'static [&'static str],
    aliases: &'static [&'static str],
    highlights: &'static str,
    injections: &'static str,
}
impl Entry {
    fn spec(&self) -> LanguageSpec {
        LanguageSpec {
            id: self.id,
            name: self.name,
            language: (self.grammar)(),
            highlights: self.highlights,
            injections: self.injections,
        }
    }
}

static LANGUAGES: &[Entry] = &[
    Entry {
        id: LanguageId::Rust,
        name: "rust",
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        extensions: &["rs"],
        filenames: &[],
        interpreters: &[],
        aliases: &["rust", "rs"],
        highlights: include_str!("../queries/rust/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Python,
        name: "python",
        grammar: || tree_sitter_python::LANGUAGE.into(),
        extensions: &["py", "pyi", "pyw"],
        filenames: &[],
        interpreters: &["python", "python3"],
        aliases: &["python", "py", "python3"],
        highlights: include_str!("../queries/python/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::JavaScript,
        name: "javascript",
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
        interpreters: &["node", "nodejs"],
        aliases: &["javascript", "js", "node"],
        highlights: include_str!("../queries/javascript/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::TypeScript,
        name: "typescript",
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        interpreters: &[],
        aliases: &["typescript", "ts"],
        highlights: include_str!("../queries/typescript/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Tsx,
        name: "tsx",
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        extensions: &["tsx"],
        filenames: &[],
        interpreters: &[],
        aliases: &["tsx"],
        highlights: include_str!("../queries/tsx/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Go,
        name: "go",
        grammar: || tree_sitter_go::LANGUAGE.into(),
        extensions: &["go"],
        filenames: &[],
        interpreters: &[],
        aliases: &["go", "golang"],
        highlights: include_str!("../queries/go/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::C,
        name: "c",
        grammar: || tree_sitter_c::LANGUAGE.into(),
        extensions: &["c", "h"],
        filenames: &[],
        interpreters: &[],
        aliases: &["c"],
        highlights: include_str!("../queries/c/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Cpp,
        name: "cpp",
        grammar: || tree_sitter_cpp::LANGUAGE.into(),
        extensions: &["cpp", "cc", "cp", "cxx", "hpp", "hh", "hxx", "ino", "tpp"],
        filenames: &[],
        interpreters: &[],
        aliases: &["cpp", "c++", "cxx"],
        highlights: include_str!("../queries/cpp/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Json,
        name: "json",
        grammar: || tree_sitter_json::LANGUAGE.into(),
        extensions: &["json", "jsonc"],
        filenames: &[],
        interpreters: &[],
        aliases: &["json", "jsonc"],
        highlights: include_str!("../queries/json/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Bash,
        name: "bash",
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        extensions: &["sh", "bash", "zsh", "ksh"],
        filenames: &[
            ".bashrc",
            ".bash_profile",
            ".bash_aliases",
            ".bash_logout",
            ".zshrc",
            ".zshenv",
            ".zprofile",
            ".profile",
            "PKGBUILD",
            "APKBUILD",
        ],
        interpreters: &["bash", "sh", "dash", "zsh", "ksh"],
        aliases: &["bash", "sh", "shell", "zsh", "shell-script"],
        highlights: include_str!("../queries/bash/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Fish,
        name: "fish",
        grammar: tree_sitter_fish::language,
        extensions: &["fish"],
        filenames: &[],
        interpreters: &["fish"],
        aliases: &["fish"],
        highlights: include_str!("../queries/fish/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Lua,
        name: "lua",
        grammar: || tree_sitter_lua::LANGUAGE.into(),
        extensions: &["lua"],
        filenames: &[],
        interpreters: &["lua"],
        aliases: &["lua"],
        highlights: include_str!("../queries/lua/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Sql,
        name: "sql",
        grammar: || tree_sitter_sequel::LANGUAGE.into(),
        extensions: &["sql"],
        filenames: &[],
        interpreters: &[],
        aliases: &["sql"],
        highlights: include_str!("../queries/sql/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::CMake,
        name: "cmake",
        grammar: || tree_sitter_cmake::LANGUAGE.into(),
        extensions: &["cmake"],
        filenames: &["CMakeLists.txt"],
        interpreters: &["cmake"],
        aliases: &["cmake"],
        highlights: include_str!("../queries/cmake/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Markdown,
        name: "markdown",
        grammar: || tree_sitter_md::LANGUAGE.into(),
        extensions: &["md", "markdown"],
        filenames: &[],
        interpreters: &[],
        aliases: &["markdown", "md"],
        highlights: include_str!("../queries/markdown/highlights.scm"),
        injections: include_str!("../queries/markdown/injections.scm"),
    },
    Entry {
        id: LanguageId::MarkdownInline,
        name: "markdown.inline",
        grammar: || tree_sitter_md::INLINE_LANGUAGE.into(),
        extensions: &[],
        filenames: &[],
        interpreters: &[],
        aliases: &["markdown.inline", "markdown_inline"],
        highlights: include_str!("../queries/markdown.inline/highlights.scm"),
        injections: include_str!("../queries/markdown.inline/injections.scm"),
    },
    Entry {
        id: LanguageId::Java,
        name: "java",
        grammar: || tree_sitter_java::LANGUAGE.into(),
        extensions: &["java"],
        filenames: &[],
        interpreters: &[],
        aliases: &["java"],
        highlights: include_str!("../queries/java/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::CSharp,
        name: "c-sharp",
        grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
        extensions: &["cs", "csx"],
        filenames: &[],
        interpreters: &[],
        aliases: &["csharp", "c#", "cs", "c-sharp"],
        highlights: include_str!("../queries/c-sharp/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Ruby,
        name: "ruby",
        grammar: || tree_sitter_ruby::LANGUAGE.into(),
        extensions: &["rb", "rbw", "rake", "gemspec"],
        filenames: &["Gemfile", "Rakefile", "Vagrantfile", "Guardfile"],
        interpreters: &["ruby"],
        aliases: &["ruby", "rb"],
        highlights: include_str!("../queries/ruby/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Php,
        name: "php",
        grammar: || tree_sitter_php::LANGUAGE_PHP.into(),
        extensions: &["php", "phtml"],
        filenames: &[],
        interpreters: &["php"],
        aliases: &["php"],
        highlights: include_str!("../queries/php/highlights.scm"),
        injections: include_str!("../queries/php/injections.scm"),
    },
    Entry {
        id: LanguageId::Toml,
        name: "toml",
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        extensions: &["toml"],
        filenames: &[],
        interpreters: &[],
        aliases: &["toml"],
        highlights: include_str!("../queries/toml/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Yaml,
        name: "yaml",
        grammar: || tree_sitter_yaml::LANGUAGE.into(),
        extensions: &["yaml", "yml"],
        filenames: &[],
        interpreters: &[],
        aliases: &["yaml", "yml"],
        highlights: include_str!("../queries/yaml/highlights.scm"),
        injections: "",
    },
    Entry {
        id: LanguageId::Html,
        name: "html",
        grammar: || tree_sitter_html::LANGUAGE.into(),
        extensions: &["html", "htm", "xhtml"],
        filenames: &[],
        interpreters: &[],
        aliases: &["html"],
        highlights: include_str!("../queries/html/highlights.scm"),
        injections: include_str!("../queries/html/injections.scm"),
    },
    Entry {
        id: LanguageId::Css,
        name: "css",
        grammar: || tree_sitter_css::LANGUAGE.into(),
        extensions: &["css"],
        filenames: &[],
        interpreters: &[],
        aliases: &["css"],
        highlights: include_str!("../queries/css/highlights.scm"),
        injections: "",
    },
];

pub fn for_extension(extension: &str) -> Option<LanguageSpec> {
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    LANGUAGES
        .iter()
        .find(|entry| {
            entry
                .extensions
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
        .map(Entry::spec)
}

pub fn for_name(name: &str) -> Option<LanguageSpec> {
    let name = name.trim();
    let name = name
        .strip_prefix("source.")
        .or_else(|| name.strip_prefix("text."))
        .unwrap_or(name);
    LANGUAGES
        .iter()
        .find(|entry| {
            entry
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
        })
        .map(Entry::spec)
}

/// A registry walk for compatibility checks, not another detection vocabulary.
pub fn specifications() -> impl Iterator<Item = LanguageSpec> {
    LANGUAGES.iter().map(Entry::spec)
}

pub fn interpreter_of(first_line: &str) -> Option<&str> {
    let mut tokens = first_line.strip_prefix("#!")?.split_whitespace();
    let program = tokens.next()?;
    let program = if basename(program) == Some("env") {
        tokens.find(|token| !token.starts_with('-'))?
    } else {
        program
    };
    basename(program)
}
fn basename(program: &str) -> Option<&str> {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
}
pub fn for_shebang(first_line: &str) -> Option<LanguageSpec> {
    let interpreter = interpreter_of(first_line)?;
    LANGUAGES
        .iter()
        .find(|entry| entry.interpreters.contains(&interpreter))
        .map(Entry::spec)
}

/// Exact basename wins over extension, which wins over the bounded shebang.
/// Non-UTF-8 filename bytes never become a lossy path used for I/O.
pub fn detect(path: &Path, first_line: Option<&str>) -> Option<LanguageSpec> {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        if let Some(entry) = LANGUAGES.iter().find(|entry| {
            entry
                .filenames
                .iter()
                .any(|filename| filename.eq_ignore_ascii_case(name))
        }) {
            return Some(entry.spec());
        }
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(for_extension)
        .or_else(|| first_line.and_then(for_shebang))
}

#[cfg(test)]
mod tests;
