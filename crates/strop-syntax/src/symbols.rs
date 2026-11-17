//! 0063 §2 syntax-definition fallback: declaration extraction over the
//! statically linked grammars (Rust, Python, C, C++, Lua, Go, Java,
//! JavaScript, TypeScript/TSX, C#, PHP, Ruby — 0107 §9). This is the
//! bounded, honest tier — useful when a semantic server is unavailable
//! or still loading — never a promise of semantic truth: macro-expanded
//! and inferred declarations are not found.
//!
//! Every extracted declaration carries its full line span (first
//! through last line, both 1-based), so a `kind:` atom can decide
//! whether a candidate line lies inside a declaration of that kind.

use crate::languages::LanguageId;
use streaming_iterator::StreamingIterator;
use strop_core::symbol_kind::SymbolKind;
use tree_sitter::{Parser, Query, QueryCursor};

/// One extracted declaration: kind, name and inclusive line span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub kind: SymbolKind,
    pub name: String,
    /// 1-based first line of the declaration's span.
    pub line: usize,
    /// 1-based last line of the span (inclusive).
    pub end_line: usize,
    /// 1-based byte column of the name (jump target, rg convention).
    pub col: usize,
}

/// One language's extraction state: parser and compiled query are
/// reused across files by the symbol index.
pub struct DeclExtractor {
    language: LanguageId,
    parser: Parser,
    query: Query,
}

impl DeclExtractor {
    /// Whether this build can extract `language`: a table lookup, no
    /// parser or query is built (0099: asked on the event loop).
    pub fn supports(language: LanguageId) -> bool {
        Self::grammar(language).is_some()
    }

    /// The linked grammar and its symbols query.
    fn grammar(language: LanguageId) -> Option<(tree_sitter::Language, &'static str)> {
        Some(match language {
            LanguageId::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                include_str!("../queries/rust/symbols.scm"),
            ),
            LanguageId::Python => (
                tree_sitter_python::LANGUAGE.into(),
                include_str!("../queries/python/symbols.scm"),
            ),
            LanguageId::C => (
                tree_sitter_c::LANGUAGE.into(),
                include_str!("../queries/c/symbols.scm"),
            ),
            LanguageId::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                include_str!("../queries/cpp/symbols.scm"),
            ),
            LanguageId::Lua => (
                tree_sitter_lua::LANGUAGE.into(),
                include_str!("../queries/lua/symbols.scm"),
            ),
            LanguageId::Go => (
                tree_sitter_go::LANGUAGE.into(),
                include_str!("../queries/go/symbols.scm"),
            ),
            LanguageId::Java => (
                tree_sitter_java::LANGUAGE.into(),
                include_str!("../queries/java/symbols.scm"),
            ),
            LanguageId::JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                include_str!("../queries/javascript/symbols.scm"),
            ),
            LanguageId::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                include_str!("../queries/typescript/symbols.scm"),
            ),
            LanguageId::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                include_str!("../queries/typescript/symbols.scm"),
            ),
            LanguageId::CSharp => (
                tree_sitter_c_sharp::LANGUAGE.into(),
                include_str!("../queries/c-sharp/symbols.scm"),
            ),
            LanguageId::Php => (
                tree_sitter_php::LANGUAGE_PHP.into(),
                include_str!("../queries/php/symbols.scm"),
            ),
            LanguageId::Ruby => (
                tree_sitter_ruby::LANGUAGE.into(),
                include_str!("../queries/ruby/symbols.scm"),
            ),
            _ => return None,
        })
    }

    /// The languages this build can extract. `None` for everything
    /// else — the index then simply has no entry for those files.
    pub fn for_language(language: LanguageId) -> Option<Self> {
        let (ts, source) = Self::grammar(language)?;
        let mut parser = Parser::new();
        parser
            .set_language(&ts)
            .map_err(|error| format!("static grammar rejected: {error}"))
            .ok()?;
        let query = Query::new(&ts, source)
            .map_err(|error| format!("static symbols query rejected: {error}"))
            .ok()?;
        Some(Self {
            language,
            parser,
            query,
        })
    }

    /// Extract declarations from one source. `None` means no evidence
    /// — the parse failed or was cancelled — which stays Unknown at
    /// admission (0063 §4); an empty `Some` is real evidence that the
    /// file declares nothing.
    pub fn extract(
        &mut self,
        source: &[u8],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<Declaration>> {
        let mut progress = |_: &tree_sitter::ParseState| cancelled();
        let tree = self.parser.parse_with_options(
            &mut |offset: usize, _| source.get(offset..).unwrap_or(&[]),
            None,
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
        )?;
        if cancelled() {
            return None;
        }
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, root, source);
        let names = self.query.capture_names();
        let mut found: Vec<Declaration> = Vec::new();
        while let Some(matched) = matches.next() {
            if cancelled() {
                return None;
            }
            let mut kind = None;
            let mut decl = None;
            let mut name = None;
            let mut implemented = None;
            for capture in matched.captures {
                match names[capture.index as usize] {
                    "name" => name = Some(capture.node),
                    "trait" => implemented = Some(capture.node),
                    other => {
                        if let Some(value) = other.strip_suffix(".decl") {
                            kind = Some(kind_of(value));
                            decl = Some(capture.node);
                        }
                    }
                }
            }
            let (Some(kind), Some(decl), Some(name)) = (kind, decl, name) else {
                continue;
            };
            let mut kind = kind;
            if kind == SymbolKind::Function && (self.lua_method(name) || self.member_context(decl))
            {
                kind = SymbolKind::Method;
            }
            let text_of = |node: tree_sitter::Node<'_>| {
                source
                    .get(node.byte_range())
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            };
            let (text, name_col) = if kind == SymbolKind::Impl {
                // named as rust-analyzer names it: `impl Type`,
                // `impl Trait for Type`, one line
                let target = text_of(name).map(|text| one_line(&text));
                let text = match implemented.map(text_of) {
                    Some(Some(implemented)) => {
                        target.map(|target| format!("impl {} for {target}", one_line(&implemented)))
                    }
                    _ => target.map(|target| format!("impl {target}")),
                };
                (text, decl.start_position().column)
            } else {
                (text_of(name), name.start_position().column)
            };
            let (Some(name), start, end) = (text, decl.start_position(), decl.end_position())
            else {
                continue;
            };
            // The span ends on the closing token's line; a node ending
            // at column 0 does not cover that line's content.
            let end_row = if end.column == 0 {
                end.row.saturating_sub(1)
            } else {
                end.row
            };
            found.push(Declaration {
                kind,
                name,
                line: start.row + 1,
                end_line: end_row + 1,
                col: name_col.saturating_add(1),
            });
        }
        found.sort_by_key(|declaration| (declaration.line, declaration.end_line));
        Some(found)
    }

    /// Lua method syntax: `function a:b()` names are
    /// method_index_expression nodes.
    fn lua_method(&self, name: tree_sitter::Node<'_>) -> bool {
        self.language == LanguageId::Lua && name.kind() == "method_index_expression"
    }

    /// A function nested inside a member container (Rust impl/trait,
    /// Python class, C/C++ class/struct body) classifies as a method.
    fn member_context(&self, decl: tree_sitter::Node<'_>) -> bool {
        let container: &[&str] = match self.language {
            LanguageId::Rust => &["impl_item", "trait_item"],
            LanguageId::Python => &["class_definition"],
            LanguageId::C | LanguageId::Cpp => &["class_specifier", "struct_specifier"],
            LanguageId::Ruby => &["class", "module", "singleton_class"],
            _ => &[],
        };
        let mut node = decl.parent();
        while let Some(parent) = node {
            if container.contains(&parent.kind()) {
                return true;
            }
            node = parent.parent();
        }
        false
    }
}

/// Source text on one line: every run of whitespace is one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn kind_of(capture: &str) -> SymbolKind {
    match capture {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "impl" => SymbolKind::Impl,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "interface" => SymbolKind::Interface,
        "module" => SymbolKind::Module,
        _ => SymbolKind::Constant,
    }
}
