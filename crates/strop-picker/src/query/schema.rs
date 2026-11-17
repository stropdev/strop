//! What each list accepts (0109 §3.2b): the one declaration per surface.
//! The parser rejects a key the surface does not declare; the hint row,
//! the footer, Ctrl-Space, the help and the docs are all read from here,
//! so nothing advertises a filter a list cannot apply.

use crate::Kind;

/// One filter key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Language,
    Path,
    FuzzyPath,
    Glob,
    Kind,
    Repo,
    Hidden,
    Ignored,
    Case,
    Text,
    Regex,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Language,
        Field::Path,
        Field::FuzzyPath,
        Field::Glob,
        Field::Kind,
        Field::Repo,
        Field::Hidden,
        Field::Ignored,
        Field::Case,
        Field::Text,
        Field::Regex,
    ];

    /// What is typed before the colon.
    pub fn key(self) -> &'static str {
        match self {
            Field::Language => "language",
            Field::Path => "path",
            Field::FuzzyPath => "fpath",
            Field::Glob => "glob",
            Field::Kind => "kind",
            Field::Repo => "repo",
            Field::Hidden => "hidden",
            Field::Ignored => "ignored",
            Field::Case => "case",
            Field::Text => "text",
            Field::Regex => "regex",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|field| field.key() == key)
    }

    /// A complete example, as the hint row and the help show it.
    pub fn example(self) -> &'static str {
        match self {
            Field::Language => "language:rust",
            Field::Path => "path:src/",
            Field::FuzzyPath => "fpath:modules/web/",
            Field::Glob => "glob:\"**/*.rs\"",
            Field::Kind => "kind:fn",
            Field::Repo => "repo:NAME",
            Field::Hidden => "hidden:include",
            Field::Ignored => "ignored:include",
            Field::Case => "case:sensitive",
            Field::Text => "text:\"...\"",
            Field::Regex => "regex:\"...\"",
        }
    }

    /// One line on what the key does.
    pub fn help(self) -> &'static str {
        match self {
            Field::Language => "The file's language (alias: language:rs)",
            Field::Path => "Text in the workspace-relative path",
            Field::FuzzyPath => "The workspace-relative path, matched fuzzily",
            Field::Glob => "A glob over the workspace-relative path; quote values with spaces",
            Field::Kind => "What the symbol is: fn, method, struct, trait, field, … (kind:fn)",
            Field::Repo => "Only the project (repository) of that name",
            Field::Hidden => "include|exclude dotfiles (default: included)",
            Field::Ignored => "include|exclude files your ignore rules skip (default: skipped)",
            Field::Case => "sensitive|ignore|smart; the default is smart case",
            Field::Text => "Literal text, even if it looks like a filter",
            Field::Regex => "A Rust regular expression",
        }
    }

    /// `-key:` excludes instead of including.
    pub fn negatable(self) -> bool {
        matches!(
            self,
            Field::Language
                | Field::Path
                | Field::FuzzyPath
                | Field::Glob
                | Field::Kind
                | Field::Repo
        )
    }
}

/// How bare words match on a surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bare {
    /// Fuzzy over a path, the file name first.
    FuzzyPath,
    /// Fuzzy over the row.
    Fuzzy,
    /// Exactly the typed text (content search).
    Phrase,
}

impl Bare {
    pub fn describe(self) -> &'static str {
        match self {
            Bare::FuzzyPath => "bare words are fuzzy paths",
            Bare::Fuzzy => "bare words are fuzzy",
            Bare::Phrase => "bare words are literal content",
        }
    }
}

/// One surface's declaration. No fields: a plain list whose field is
/// never parsed (action titles hold colons; a branch name is a name).
#[derive(Debug, PartialEq, Eq)]
pub struct Schema {
    pub fields: &'static [Field],
    pub bare: Bare,
    /// What the rows are, as an error names them: "kind: does not filter files".
    pub noun: &'static str,
    /// The key that opens the surface, as the help lists it.
    pub surface: &'static str,
}

impl Schema {
    pub fn declares(&self, field: Field) -> bool {
        self.fields.contains(&field)
    }

    /// A plain list: the field is literal text, nothing is parsed.
    pub fn is_plain(&self) -> bool {
        self.fields.is_empty()
    }

    /// Every key's example, in declaration order: the empty list's
    /// teaching line.
    pub fn examples(&self) -> String {
        let examples: Vec<&str> = self.fields.iter().map(|field| field.example()).collect();
        examples.join(" ")
    }

    /// The one-line reminder under a field: the everyday keys and where
    /// the rest are. Empty for a plain list, which promises nothing.
    pub fn hint(&self) -> String {
        if self.is_plain() {
            return String::new();
        }
        let everyday: Vec<&str> = self
            .fields
            .iter()
            .filter(|field| {
                !matches!(
                    field,
                    Field::FuzzyPath | Field::Hidden | Field::Ignored | Field::Text | Field::Regex
                )
            })
            .take(4)
            .map(|field| field.example())
            .collect();
        format!("{} · ctrl-space lists all", everyday.join(" "))
    }
}

use Field as F;

const PATH_LIST: &[Field] = &[
    F::Language,
    F::Path,
    F::FuzzyPath,
    F::Glob,
    F::Case,
    F::Text,
    F::Regex,
];

pub const FILES: Schema = Schema {
    fields: &[
        F::Language,
        F::Path,
        F::FuzzyPath,
        F::Glob,
        F::Hidden,
        F::Ignored,
        F::Case,
        F::Text,
        F::Regex,
    ],
    bare: Bare::FuzzyPath,
    noun: "files",
    surface: "space f",
};
pub const SEARCH: Schema = Schema {
    fields: &Field::ALL,
    bare: Bare::Phrase,
    noun: "search hits",
    surface: "space /",
};
pub const SYMBOLS: Schema = Schema {
    fields: &[
        F::Kind,
        F::Language,
        F::Path,
        F::FuzzyPath,
        F::Glob,
        F::Case,
        F::Text,
        F::Regex,
    ],
    bare: Bare::Fuzzy,
    noun: "symbols",
    surface: "space s",
};
pub const WORKSPACE_SYMBOLS: Schema = Schema {
    fields: &[
        F::Kind,
        F::Language,
        F::Path,
        F::FuzzyPath,
        F::Glob,
        F::Hidden,
        F::Ignored,
        F::Case,
        F::Text,
        F::Regex,
    ],
    bare: Bare::Fuzzy,
    noun: "workspace symbols",
    surface: "space S",
};
pub const BUFFERS: Schema = Schema {
    fields: PATH_LIST,
    bare: Bare::FuzzyPath,
    noun: "buffers",
    surface: "space b",
};
pub const JUMPS: Schema = Schema {
    fields: PATH_LIST,
    bare: Bare::Fuzzy,
    noun: "jumps",
    surface: "space j",
};
pub const DIAGNOSTICS: Schema = Schema {
    fields: PATH_LIST,
    bare: Bare::Fuzzy,
    noun: "diagnostics",
    surface: "space d",
};
pub const LOCATIONS: Schema = Schema {
    fields: PATH_LIST,
    bare: Bare::Fuzzy,
    noun: "locations",
    surface: "grr gri",
};
/// `:filter` in a folder view.
pub const FOLDER: Schema = Schema {
    fields: &[
        F::Language,
        F::Path,
        F::FuzzyPath,
        F::Glob,
        F::Hidden,
        F::Ignored,
        F::Case,
        F::Text,
        F::Regex,
    ],
    bare: Bare::Phrase,
    noun: "folder entries",
    surface: ":filter",
};
pub const PLAIN: Schema = Schema {
    fields: &[],
    bare: Bare::Fuzzy,
    noun: "rows",
    surface: "",
};

/// Every surface that takes filters, in the order help lists them.
pub const SURFACES: [&Schema; 9] = [
    &FILES,
    &SEARCH,
    &SYMBOLS,
    &WORKSPACE_SYMBOLS,
    &BUFFERS,
    &JUMPS,
    &DIAGNOSTICS,
    &LOCATIONS,
    &FOLDER,
];

impl Kind {
    pub fn schema(self) -> &'static Schema {
        match self {
            Kind::Files => &FILES,
            Kind::Search => &SEARCH,
            Kind::Symbols => &SYMBOLS,
            Kind::WorkspaceSymbols => &WORKSPACE_SYMBOLS,
            Kind::Buffers => &BUFFERS,
            Kind::Jumps => &JUMPS,
            Kind::Diagnostics => &DIAGNOSTICS,
            Kind::Locations => &LOCATIONS,
            Kind::RemoteHosts
            | Kind::RemoteAddress
            | Kind::Actions
            | Kind::TabSize
            | Kind::Theme
            | Kind::Branches => &PLAIN,
        }
    }
}

/// The surfaces that take `field`, as the help and errors name them.
pub fn surfaces_of(field: Field) -> impl Iterator<Item = &'static str> {
    SURFACES
        .into_iter()
        .filter(move |schema| schema.declares(field))
        .map(|schema| schema.surface)
}
