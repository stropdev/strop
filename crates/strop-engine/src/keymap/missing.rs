//! What Neovim binds and strop does not (0095 §2.7). Two lists, both
//! printed by `strop compat` under the binding table:
//!
//! - `MISSING`: the key does nothing here. A test feeds every row and
//!   fails if one starts to work — it then belongs in the table.
//! - `REASSIGNED`: strop uses the key for something else.
//!
//! A Neovim user reads these to learn what their hands will miss; they
//! are the honest half of the compatibility page.

/// Where a missing key would be typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    Normal,
    Visual,
    Insert,
    /// The `:` line; the row's keys are command lines.
    Ex,
}

impl Where {
    pub fn title(self) -> &'static str {
        match self {
            Where::Normal => "Normal",
            Where::Visual => "Visual",
            Where::Insert => "Insert",
            Where::Ex => "Ex",
        }
    }
}

/// One group of keys Neovim binds and strop does not.
pub struct Missing {
    pub place: Where,
    /// Space-separated key sequences in the binding table's notation
    /// (`ctrl-w x`), or `:command` lines for `Where::Ex`.
    pub keys: &'static [&'static str],
    pub what: &'static str,
}

const fn missing(place: Where, keys: &'static [&'static str], what: &'static str) -> Missing {
    Missing { place, keys, what }
}

pub const MISSING: &[Missing] = &[
    missing(
        Where::Normal,
        &["zfj", "zo", "zc", "za", "zR", "zM", "zj", "zk", "zd"],
        "folds",
    ),
    missing(Where::Normal, &["gt", "gT"], "tab pages"),
    missing(Where::Normal, &["U"], "undo every change on the line"),
    missing(Where::Normal, &["==", "=j", "=ip"], "the reindent operator"),
    missing(
        Where::Normal,
        &["gqq", "gqip", "gwip"],
        "reflow text to the text width",
    ),
    missing(Where::Normal, &["dis", "das"], "the sentence text objects"),
    missing(Where::Normal, &["q:", "q/"], "the command-line window"),
    missing(
        Where::Normal,
        &["ga", "g8"],
        "show the character under the cursor as a code",
    ),
    missing(
        Where::Normal,
        &["gx", "gf", "gF"],
        "open the URL or file name under the cursor",
    ),
    missing(Where::Normal, &["gR"], "virtual replace mode"),
    missing(
        Where::Normal,
        &["K"],
        "look the word under the cursor up (hover is `space k`)",
    ),
    missing(
        Where::Normal,
        &["]]", "[[", "][", "[]", "]m", "[m"],
        "section and method motions",
    ),
    missing(Where::Normal, &["z=", "zg", "]s", "[s"], "spelling"),
    missing(
        Where::Normal,
        &["gm", "gM", "go"],
        "middle of the line, byte offset",
    ),
    missing(
        Where::Normal,
        &["zh", "zl", "zH", "zL", "ze", "zs"],
        "horizontal scrolling",
    ),
    missing(Where::Normal, &["do", "dp"], "diff mode"),
    missing(
        Where::Normal,
        &[
            "ctrl-w x", "ctrl-w r", "ctrl-w T", "ctrl-w H", "ctrl-w J", "ctrl-w K", "ctrl-w L",
            "ctrl-w +", "ctrl-w -", "ctrl-w _", "ctrl-w |",
        ],
        "move, rotate and resize windows (panes are always equal)",
    ),
    missing(
        Where::Visual,
        &["=", "gq", "gw", "zf"],
        "reindent, reflow and fold the selection",
    ),
    missing(
        Where::Insert,
        &["ctrl-v", "ctrl-k", "ctrl-a"],
        "insert a literal key, a digraph, the last inserted text",
    ),
    missing(
        Where::Ex,
        &[
            ":tabnew",
            ":tabnext",
            ":tabclose",
            ":fold",
            ":mksession",
            ":digraphs",
        ],
        "tab pages, folds, session files, digraphs",
    ),
];

/// A key Neovim binds that means something else in strop.
pub struct Reassigned {
    pub place: Where,
    pub keys: &'static str,
    pub neovim: &'static str,
    pub strop: &'static str,
}

const fn reassigned(
    place: Where,
    keys: &'static str,
    neovim: &'static str,
    strop: &'static str,
) -> Reassigned {
    Reassigned {
        place,
        keys,
        neovim,
        strop,
    }
}

pub const REASSIGNED: &[Reassigned] = &[
    reassigned(Where::Normal, "space", "move right", "the leader"),
    reassigned(
        Where::Normal,
        "Q",
        "replay the last recorded register",
        "toggle a cursor here (`@@` replays)",
    ),
    reassigned(
        Where::Normal,
        "gd gD",
        "go to the local or global declaration by search",
        "language-server definition and declaration",
    ),
    reassigned(
        Where::Normal,
        "gI",
        "insert in column 1",
        "language-server implementation",
    ),
    reassigned(
        Where::Normal,
        "gr",
        "replace with a virtual character (`gr{char}`)",
        "only the prefix of the language-server keys `grr` `grn` `gra` `gri`",
    ),
    reassigned(
        Where::Normal,
        "gs",
        "sleep",
        "switch between source and header",
    ),
    reassigned(
        Where::Normal,
        "ds cs ys",
        "nothing (`s` is not a motion)",
        "delete, change, add a surrounding pair",
    ),
    reassigned(
        Where::Visual,
        "S",
        "change the selected lines",
        "wrap the selection in a pair (`C` and `R` change lines)",
    ),
    reassigned(
        Where::Visual,
        "ctrl-c",
        "leave Visual",
        "toggle comments on the selected lines (`Esc` leaves)",
    ),
    reassigned(
        Where::Insert,
        "ctrl-e ctrl-y",
        "insert the character below or above",
        "dismiss or accept the completion menu",
    ),
];
