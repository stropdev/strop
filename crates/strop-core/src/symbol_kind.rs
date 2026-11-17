//! The one symbol-kind table (0109 §3.2a): what a declaration is, the
//! chip a list shows for it, and every word `kind:` accepts for it.
//! Language servers, the syntax outline, the query language, Ctrl-Space
//! suggestions and the help all read this table; there is no second
//! vocabulary.

/// What a declaration is. Every LSP `SymbolKind` maps to one of these
/// (see [`KindSpec::lsp`]); a kind a future protocol adds is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind {
    Function,
    /// A function in a member context (impl/trait, class body).
    Method,
    Constructor,
    Class,
    Struct,
    Interface,
    /// A Rust `impl` block (rust-analyzer reports it as `Object`).
    Impl,
    Enum,
    /// An enum member.
    Variant,
    Module,
    File,
    Field,
    Property,
    Variable,
    Constant,
    /// A type parameter; rust-analyzer also reports type aliases so.
    Type,
    Event,
    Operator,
    Key,
    /// A literal value in a data file (string, number, boolean, array, null).
    Value,
    Other,
}

/// One row of the table.
#[derive(Debug, PartialEq, Eq)]
pub struct KindSpec {
    pub kind: SymbolKind,
    /// The canonical `kind:` value, also what Ctrl-Space offers.
    pub value: &'static str,
    /// The row badge.
    pub chip: &'static str,
    /// Other words `kind:` accepts: common language spellings.
    pub aliases: &'static [&'static str],
    /// The LSP `SymbolKind` names that are this kind.
    pub lsp: &'static [&'static str],
    /// What `kind:<value>` admits: this kind first, then any it covers.
    pub includes: &'static [SymbolKind],
}

use SymbolKind as K;

/// Every kind, in the order help and suggestions list them: kind,
/// value, chip, other words, LSP names, what `kind:<value>` admits.
#[rustfmt::skip]
pub const KINDS: &[KindSpec] = &[
    spec(K::Function, "function", "fn", &["func", "fun", "def", "sub", "proc"], &["Function"], &[K::Function, K::Method, K::Constructor]),
    spec(K::Method, "method", "meth", &[], &["Method"], &[K::Method]),
    spec(K::Constructor, "constructor", "ctor", &["new", "init"], &["Constructor"], &[K::Constructor]),
    spec(K::Class, "class", "class", &[], &["Class"], &[K::Class]),
    spec(K::Struct, "struct", "struct", &["record"], &["Struct"], &[K::Struct]),
    spec(K::Interface, "interface", "iface", &["trait", "protocol"], &["Interface"], &[K::Interface]),
    spec(K::Impl, "impl", "impl", &["object"], &["Object"], &[K::Impl]),
    spec(K::Enum, "enum", "enum", &[], &["Enum"], &[K::Enum]),
    spec(K::Variant, "variant", "variant", &["enummember"], &["EnumMember"], &[K::Variant]),
    spec(K::Module, "module", "mod", &["namespace", "ns", "package", "pkg"], &["Module", "Namespace", "Package"], &[K::Module]),
    spec(K::File, "file", "file", &[], &["File"], &[K::File]),
    spec(K::Field, "field", "field", &[], &["Field"], &[K::Field]),
    spec(K::Property, "property", "prop", &["attribute", "attr"], &["Property"], &[K::Property]),
    spec(K::Variable, "variable", "var", &["let", "local"], &["Variable"], &[K::Variable]),
    spec(K::Constant, "constant", "const", &["static"], &["Constant"], &[K::Constant]),
    spec(K::Type, "type", "type", &["typeparam", "generic", "alias"], &["TypeParameter"], &[K::Type]),
    spec(K::Event, "event", "event", &[], &["Event"], &[K::Event]),
    spec(K::Operator, "operator", "op", &[], &["Operator"], &[K::Operator]),
    spec(K::Key, "key", "key", &[], &["Key"], &[K::Key]),
    spec(K::Value, "value", "val", &["string", "number", "boolean", "array", "null"], &["String", "Number", "Boolean", "Array", "Null"], &[K::Value]),
    spec(K::Other, "other", "sym", &[], &[], &[K::Other]),
];

const fn spec(
    kind: SymbolKind,
    value: &'static str,
    chip: &'static str,
    aliases: &'static [&'static str],
    lsp: &'static [&'static str],
    includes: &'static [SymbolKind],
) -> KindSpec {
    KindSpec {
        kind,
        value,
        chip,
        aliases,
        lsp,
        includes,
    }
}

impl SymbolKind {
    /// This kind's row in [`KINDS`].
    pub fn spec(self) -> &'static KindSpec {
        let mut at = 0;
        while at < KINDS.len() {
            if KINDS[at].kind == self {
                return &KINDS[at];
            }
            at += 1;
        }
        // every variant has a row (pinned by a test); `Other` is last
        &KINDS[KINDS.len() - 1]
    }

    pub fn chip(self) -> &'static str {
        self.spec().chip
    }
}

impl KindSpec {
    /// Every word that names this kind: the value, the chip, the aliases.
    pub fn words(&self) -> impl Iterator<Item = &'static str> {
        [self.value, self.chip]
            .into_iter()
            .chain(self.aliases.iter().copied())
    }

    pub fn admits(&self, kind: SymbolKind) -> bool {
        self.includes.contains(&kind)
    }
}

/// How a typed `kind:` value reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Kind(&'static KindSpec),
    /// A prefix of words naming more than one kind: their values.
    Ambiguous(Vec<&'static str>),
    Unknown,
}

/// Read a `kind:` value: any word of a kind, ignoring ASCII case; a
/// prefix is accepted when every word it starts names the same kind.
pub fn resolve(value: &str) -> Resolved {
    if value.is_empty() {
        return Resolved::Unknown;
    }
    let exact = KINDS
        .iter()
        .find(|spec| spec.words().any(|word| word.eq_ignore_ascii_case(value)));
    if let Some(spec) = exact {
        return Resolved::Kind(spec);
    }
    let starts = |word: &str| {
        word.len() > value.len()
            && word.as_bytes()[..value.len()].eq_ignore_ascii_case(value.as_bytes())
    };
    let matching: Vec<&'static KindSpec> = KINDS
        .iter()
        .filter(|spec| spec.words().any(starts))
        .collect();
    match matching.as_slice() {
        [] => Resolved::Unknown,
        [spec] => Resolved::Kind(spec),
        many => Resolved::Ambiguous(many.iter().map(|spec| spec.value).collect()),
    }
}

/// Every word `kind:` accepts, for suggestions and corrections.
pub fn kind_words() -> impl Iterator<Item = &'static str> {
    KINDS.iter().flat_map(KindSpec::words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSP_NAMES: [&str; 26] = [
        "File",
        "Module",
        "Namespace",
        "Package",
        "Class",
        "Method",
        "Property",
        "Field",
        "Constructor",
        "Enum",
        "Interface",
        "Function",
        "Variable",
        "Constant",
        "String",
        "Number",
        "Boolean",
        "Array",
        "Object",
        "Key",
        "Null",
        "EnumMember",
        "Struct",
        "Event",
        "Operator",
        "TypeParameter",
    ];

    #[test]
    fn every_lsp_symbol_kind_name_has_exactly_one_kind() {
        for name in LSP_NAMES {
            let owners = KINDS.iter().filter(|spec| spec.lsp.contains(&name)).count();
            assert_eq!(owners, 1, "{name}");
        }
        let listed: usize = KINDS.iter().map(|spec| spec.lsp.len()).sum();
        assert_eq!(listed, LSP_NAMES.len());
    }

    #[test]
    fn every_kind_has_one_row_and_includes_itself_first() {
        for spec in KINDS {
            assert_eq!(spec.kind.spec().kind, spec.kind);
            assert_eq!(spec.includes.first(), Some(&spec.kind));
        }
        assert_eq!(KINDS.last().map(|spec| spec.kind), Some(SymbolKind::Other));
    }

    #[test]
    fn every_word_is_lowercase_and_names_its_own_kind() {
        for spec in KINDS {
            for word in spec.words() {
                assert_eq!(word, word.to_ascii_lowercase());
                assert_eq!(resolve(word), Resolved::Kind(spec), "{word}");
            }
        }
        let mut words: Vec<_> = kind_words().collect();
        let total = words.len();
        words.sort_unstable();
        words.dedup();
        // a chip may equal its own value (`enum`), never another kind's word
        let distinct_per_kind: usize = KINDS
            .iter()
            .map(|spec| {
                let mut own: Vec<_> = spec.words().collect();
                own.sort_unstable();
                own.dedup();
                own.len()
            })
            .sum();
        assert_eq!(words.len(), distinct_per_kind, "{total} words");
    }

    #[test]
    fn values_ignore_case_and_take_unique_prefixes() {
        assert_eq!(resolve("Function").kinds(), Some(SymbolKind::Function));
        assert_eq!(resolve("FN").kinds(), Some(SymbolKind::Function));
        assert_eq!(resolve("trait").kinds(), Some(SymbolKind::Interface));
        assert_eq!(resolve("funct").kinds(), Some(SymbolKind::Function));
        assert_eq!(resolve("str").kinds(), None, "struct and string");
        assert_eq!(resolve("stru").kinds(), Some(SymbolKind::Struct));
        assert!(matches!(resolve("f"), Resolved::Ambiguous(values) if values.contains(&"field")));
        assert_eq!(resolve("banana"), Resolved::Unknown);
        assert_eq!(resolve(""), Resolved::Unknown);
    }

    #[test]
    fn function_includes_method_and_constructor() {
        let Resolved::Kind(function) = resolve("function") else {
            panic!("function is a kind");
        };
        for kind in [
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
        ] {
            assert!(function.admits(kind));
        }
        assert!(!function.admits(SymbolKind::Field));
        let Resolved::Kind(method) = resolve("method") else {
            panic!("method is a kind");
        };
        assert!(!method.admits(SymbolKind::Function));
    }

    impl Resolved {
        fn kinds(&self) -> Option<SymbolKind> {
            match self {
                Resolved::Kind(spec) => Some(spec.kind),
                _ => None,
            }
        }
    }
}
