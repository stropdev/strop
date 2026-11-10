//! Where strop deliberately differs from Neovim. Each entry is one
//! reason and the cases it explains (a name, or a prefix ending in `*`).
//! The list is capped: a new reason means retiring one first.

/// The corpus may never shrink below this (0095 §4).
pub const MINIMUM_CASES: usize = 600;

pub const MAXIMUM_KNOWN: usize = 10;

/// (cases, reason)
pub const KNOWN: &[(&[&str], &str)] = &[
    (
        &[
            "critique/zf",
            "critique/U",
            "critique/=ip",
            "critique/vis =",
            "critique/gqq",
            "critique/gwip",
            "critique/searchoffset",
        ],
        "On the Missing list (`strop compat`, keymap/missing.rs): folds, `U`, the `=` \
         and `gq`/`gw` operators, and search offsets (`/pat/e`). The key does \
         nothing in strop; it is listed, not silently absent.",
    ),
    (
        &["critique/gd-local", "critique/gI"],
        "On the Reassigned list: `gd` and `gI` are the language server's definition \
         and implementation. Without a server they do nothing where Neovim searches \
         the buffer or inserts in column 1.",
    ),
    (
        &["daw on line-ending blanks is a no-op"],
        "vim's FAILED current_word still moves the cursor one column (its FAIL path \
         returns after incl()); strop refuses without moving the cursor. Text agrees; \
         only the cursor side effect of a refused command differs.",
    ),
    (
        &[
            "critique/gn",
            "critique/&",
            "critique/g&",
            "critique/:s&",
            "gen/code@2:5/dot/gU/in⏎",
            "gen/code@2:5/dot/gU?in⏎",
            "gen/code@4:14/dot/g~/in⏎",
            "gen/code@4:14/dot/g~?in⏎",
        ],
        "Search is smart case (plan 0093 §3.7), in `/`, `?` and `:s` alike: an \
         all-lowercase pattern matches either case, so `/edge` also finds `Edge` and \
         `/in` still finds the `IN` an operator has just upper-cased. `nvim -u NONE` \
         is case-sensitive. These cases search lower-case for text that also occurs \
         in another case; a pattern that spells a capital agrees with Neovim.",
    ),
];

/// The index of the entry that explains this case, if one does.
pub fn reason(name: &str) -> Option<usize> {
    KNOWN.iter().position(|(cases, _)| {
        cases.iter().any(|case| match case.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *case,
        })
    })
}
