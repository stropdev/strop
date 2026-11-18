//! What the help says beyond the binding table and the command registry
//! (0112 §8): the hand-written rows as typed data — each key cell holds
//! only what is typed, or is a `Label` — and the rows generated from the
//! query schema and the LSP registry.

use super::page::Cell;
use crate::editor::ex::Family;

/// One hand-written row: what its key cell is, the cell, the words.
pub(super) struct Row {
    pub cell: Cell,
    pub keys: &'static str,
    pub desc: &'static str,
}

const fn keys(keys: &'static str, desc: &'static str) -> Row {
    Row {
        cell: Cell::Keys,
        keys,
        desc,
    }
}

const fn command(keys: &'static str, desc: &'static str) -> Row {
    Row {
        cell: Cell::Command,
        keys,
        desc,
    }
}

const fn filter(keys: &'static str, desc: &'static str) -> Row {
    Row {
        cell: Cell::Filter,
        keys,
        desc,
    }
}

const fn label(keys: &'static str, desc: &'static str) -> Row {
    Row {
        cell: Cell::Label,
        keys,
        desc,
    }
}

/// The query language's rows the schema does not generate: how text is
/// read, quoted and replaced. Every key's row comes from the schema
/// (0109, see [`query_language`]).
const QUERY_NOTES: &[Row] = &[
    filter("-language: -kind: -path:", "Exclude; exclusions win"),
    keys(
        "AND OR NOT ( )",
        "Combine filters and words; bare words beside a filter stay fuzzy",
    ),
    keys(
        "ctrl-space",
        "Suggest the filters and values this list takes",
    ),
    label(
        "unknown filter",
        "An error under the word; the list stays as it was until it is fixed",
    ),
    label(
        "scope",
        "Regular local files; directory links and special files are not searched",
    ),
    label(
        "SSH listings",
        "Hidden entries included; no ignore-file filtering (local search defaults do not apply)",
    ),
    label(
        "filter-only Search",
        "Needs a search expression; file find lists matches",
    ),
    label(
        "quoted text",
        "Stays literal — never a filter (\"language:rust\" searches that text)",
    ),
    label(
        "old -t / -g flags",
        "Now ordinary literal text; use language: and glob:",
    ),
    label(
        "file expressions",
        "Bare words are fuzzy; quoted/text: are literal; regex: is explicit",
    ),
    label(
        "quotes",
        "Single/double quotes; escape the active quote or backslash",
    ),
    label(
        "With",
        "Literal text; after a regex: term with groups, $1 ${name} \\1 expand",
    ),
    command(
        ":[range]s/a/b/[g]",
        "Buffer-search pattern; live before/after preview before Enter",
    ),
    label(
        ":s replacement",
        "Vim's: & \\1 ~ \\r \\u \\U expand; \\& and \\\\ are themselves",
    ),
    keys(
        "/ ?",
        "Buffer search: smart case by default; case:sensitive or \\C forces exact case",
    ),
    filter(
        "case:ignore",
        "Buffer search: explicit insensitive search (alias: case:insensitive)",
    ),
    keys(
        "ctrl-r {reg}",
        "Insert a register in any input: insert mode, : / ?, Files, Search, With",
    ),
];

/// One generated row: the cell, the words.
pub(super) struct Generated {
    pub cell: Cell,
    pub keys: String,
    pub desc: String,
}

/// "Query language": one row per filter key with the lists that take
/// it, one per `kind:` value, then the notes — generated from the
/// schema and the kind table, so the help cannot promise what a list
/// ignores.
pub(super) fn query_language() -> Vec<Generated> {
    use strop_picker::query::schema::{surfaces_of, Field};
    let mut rows: Vec<Generated> = Field::ALL
        .into_iter()
        .map(|field| {
            let surfaces: Vec<&str> = surfaces_of(field).collect();
            Generated {
                cell: Cell::Filter,
                keys: field.example().to_owned(),
                desc: format!("{} · {}", field.help(), surfaces.join(", ")),
            }
        })
        .collect();
    for spec in strop_core::symbol_kind::KINDS {
        let words: Vec<&str> = spec.words().filter(|word| *word != spec.value).collect();
        let mut parts = Vec::new();
        if !words.is_empty() {
            parts.push(format!("also {}", words.join(", ")));
        }
        if !spec.lsp.is_empty() {
            parts.push(format!("language servers' {}", spec.lsp.join(", ")));
        }
        if parts.is_empty() {
            parts.push("symbols no other kind names".into());
        }
        rows.push(Generated {
            cell: Cell::Filter,
            keys: format!("kind:{}", spec.value),
            desc: parts.join(" · "),
        });
    }
    rows.extend(QUERY_NOTES.iter().map(|row| Generated {
        cell: row.cell,
        keys: row.keys.to_owned(),
        desc: row.desc.to_owned(),
    }));
    rows
}

/// Which server each language family (the `language:` values) starts,
/// and how to get it; families without one share the last row, whose
/// key cell is empty.
pub(super) fn lsp_servers() -> Vec<Generated> {
    let mut rows = Vec::new();
    let mut serverless = Vec::new();
    for language in strop_core::languages::language_names() {
        match strop_lsp::registry::embedded_server(language) {
            Some((server, hint)) => rows.push(Generated {
                cell: Cell::Label,
                keys: language.to_owned(),
                desc: format!("{server} · {hint}"),
            }),
            None => serverless.push(language),
        }
    }
    if !serverless.is_empty() {
        rows.push(Generated {
            cell: Cell::Label,
            keys: String::new(),
            desc: format!(
                "no built-in server: {} (config: [language.NAME])",
                serverless.join(", ")
            ),
        });
    }
    rows
}

pub(super) const REMOTE: &[Row] = &[
    command(
        "strop ssh://host/path",
        "How to open: a remote file or directory from the shell",
    ),
    command(
        ":e ssh://host/path/",
        "How to open: a remote folder from inside the editor",
    ),
    keys(
        "space o / :remote",
        "Choose an SSH destination; Add a host… enters an address",
    ),
    command(":remote root / home", "Open the host's root or home folder"),
    command(
        ":w / :wq",
        "Save to the host; refused if the file changed since it was read",
    ),
    command(":w!", "Save to the host even if the file changed there"),
    command(
        ":w ssh://… / :w PATH",
        "Write a copy on the host; never replaces a file without !",
    ),
    command(
        ":remote connect URI",
        "Connect to a host; a connected host is editable",
    ),
    command(":remote disconnect URI", "Close the host's connection"),
    command(":remote clear", "Close all connections"),
    command(":remote list", "List connections in a buffer"),
    keys("tab", "Complete an SSH URI without starting authentication"),
    command(":e!", "Reload the remote file, dropping unsaved changes"),
    command(
        ":explain",
        "Why: workspaces, LSP readiness/failures, effective config",
    ),
];

pub(super) const SEARCH_WORKSPACE: &[Row] = &[
    keys(
        "space /",
        "Open/resume the retained local Search investigation",
    ),
    keys("space R", "Resume the same Search with replacement visible"),
    keys(
        "ctrl-t",
        "Toggle With without restarting Search or losing its draft",
    ),
    keys(
        "enter",
        "In Find: open selected source, even when With is visible",
    ),
    keys(
        "enter",
        "In With: prepare Review; never apply or save directly",
    ),
    keys(
        "tab / s-tab",
        "Switch visible fields; otherwise move results",
    ),
    keys(
        "ctrl-x / ctrl-d",
        "Exclude/include match or file in the shared workset",
    ),
    keys(
        "esc",
        "Dismiss suggestions, enter field Normal, then hide Search",
    ),
    label(
        "re-entry",
        "Refresh exact witnesses in the captured scope; report lost decisions",
    ),
    label(
        "scope",
        "Local project backend; unsupported namespaces refuse, never fall back",
    ),
];

pub(super) const COLLECTIONS: &[Row] = &[
    keys(
        "ctrl-o",
        "In search results, references or diagnostics: collect the included hits",
    ),
    keys(
        "]q / [q",
        "Next/previous hit, from the cursor; from a file, in the hit's file",
    ),
    keys("enter", "Open the file at the cursor; Ctrl-O comes back"),
    keys("]f / [f", "Next/previous file"),
    keys(
        "+ / -",
        "Show more/less context around the hits under the cursor",
    ),
    label(
        "edit",
        "Edits inside a card change its file; borders and gaps are not text",
    ),
    command(":w", "Write every changed file the collection shows"),
    keys("u", "Undo the collection's own edits, in its files too"),
    command(
        ":copen",
        "Back to the collection, on its current hit (Vim's quickfix names work)",
    ),
    command(
        ":cdo / :cfdo",
        "Run an ex command at every hit, or once per file: :cdo s/a/b/g",
    ),
];

pub(super) const INDENTATION: &[Row] = &[
    command(
        ":tab-size [N|auto]",
        "Source width, 1-16; no argument opens the selector",
    ),
    command(
        ":indent-style [spaces|tabs|auto]",
        "Source style or configured/detected policy",
    ),
    label(
        "detection",
        "Bounded open/reload sample; ambiguity keeps the configured fallback",
    ),
];

pub(super) const REVIEW: &[Row] = &[
    keys(
        "enter",
        "In With: prepare the review: the exact diff, nothing changed yet",
    ),
    keys("enter", "In the review: apply every file and write it"),
    keys(
        "u",
        "In the review: revert the buffers and the files; names what it could not",
    ),
    keys(
        "q",
        "Close the review; one that was never applied is cancelled",
    ),
    keys("$1 \\1", "With a regex: term, captures expand in With"),
];

pub(super) const DOCUMENTATION: &[Row] = &[keys(
    "enter",
    "On a hover card: open full searchable documentation; Ctrl-O returns",
)];

/// Where a part's rows come from.
pub(super) enum Source {
    /// A keymap section's bindings, by id.
    Section(&'static str),
    /// A command family's registry rows.
    Family(Family),
    /// Hand-written rows under a title.
    Rows(&'static str, &'static [Row]),
    QueryLanguage,
    LspServers,
}

impl Source {
    /// The part's `─ Title ─`.
    pub fn title(&self) -> &'static str {
        match self {
            Source::Section(id) => {
                crate::keymap::Section::find(id).map_or(id, |section| section.title)
            }
            Source::Family(family) => family.title(),
            Source::Rows(title, _) => title,
            Source::QueryLanguage => "Query language",
            Source::LspServers => "LSP servers",
        }
    }
}

/// One section of the page and its parts, in reading order (§3.1).
pub(super) struct TabSpec {
    pub title: &'static str,
    pub parts: &'static [Source],
}

/// The page's sections. `keymap::SECTIONS` keeps which-key's order;
/// this is the help's.
pub(super) const TABS: &[TabSpec] = &[
    TabSpec {
        title: "Move",
        parts: &[Source::Section("moving"), Source::Section("search")],
    },
    TabSpec {
        title: "Edit",
        parts: &[Source::Section("editing"), Source::Section("insert")],
    },
    TabSpec {
        title: "Visual",
        parts: &[Source::Section("visual")],
    },
    TabSpec {
        title: "Leader",
        parts: &[Source::Section("leader")],
    },
    TabSpec {
        title: "Windows",
        parts: &[Source::Section("panes")],
    },
    TabSpec {
        title: "Git",
        parts: &[Source::Section("git")],
    },
    TabSpec {
        title: "Lists",
        parts: &[
            Source::Section("pickers"),
            Source::Section("folder"),
            Source::Rows("Search workspace", SEARCH_WORKSPACE),
            Source::Rows("Collections", COLLECTIONS),
            Source::Rows("Review", REVIEW),
            Source::QueryLanguage,
        ],
    },
    TabSpec {
        title: "Commands",
        parts: &[
            Source::Section("cmdline"),
            Source::Family(Family::Text),
            Source::Family(Family::Buffers),
            Source::Family(Family::Files),
            Source::Family(Family::Tabs),
            Source::Family(Family::Own),
        ],
    },
    TabSpec {
        title: "More",
        parts: &[
            Source::Section("terminal"),
            Source::Rows("Remote", REMOTE),
            Source::Rows("Indentation", INDENTATION),
            Source::Rows("Documentation", DOCUMENTATION),
            Source::LspServers,
        ],
    },
];
