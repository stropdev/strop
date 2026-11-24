//! Gutter signs per change run (#240): what a diff between two texts
//! puts on which new-side line, before any row clamping.

use super::Sign::{self, Add, Change, DeleteAfter};
use crate::repo::hunks_from_strings;
use std::path::Path;

fn signs(old: &str, new: &str) -> Vec<(usize, Sign)> {
    hunks_from_strings(old, new, Path::new("note"))
        .unwrap()
        .iter()
        .flat_map(|hunk| hunk.signs())
        .collect()
}

const BASE: &str = "one\ntwo\nthree\n";

/// (name, old text, new text, expected signs)
type Case = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(usize, Sign)],
);

#[test]
fn sign_table() {
    let ten = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    let cases: &[Case] = &[
        (
            "add in the middle",
            BASE,
            "one\nnew\ntwo\nthree\n",
            &[(2, Add)],
        ),
        (
            "add at the top",
            BASE,
            "new\none\ntwo\nthree\n",
            &[(1, Add)],
        ),
        ("add at EOF", BASE, "one\ntwo\nthree\nfour\n", &[(4, Add)]),
        (
            "delete in the middle",
            BASE,
            "one\nthree\n",
            &[(2, DeleteAfter)],
        ),
        (
            "delete at the top",
            BASE,
            "two\nthree\n",
            &[(1, DeleteAfter)],
        ),
        (
            "delete at EOF, newline kept",
            BASE,
            "one\ntwo\n",
            &[(3, DeleteAfter)],
        ),
        (
            "delete at EOF, no final newline",
            "one\ntwo\nthree",
            "one\ntwo",
            // `two` lost its newline: a change run, not a bare deletion
            &[(2, Change)],
        ),
        ("delete everything", BASE, "", &[(0, DeleteAfter)]),
        (
            "one-line replacement",
            BASE,
            "one\nTWO\nthree\n",
            &[(2, Change)],
        ),
        (
            "replacement growing 1 -> 3",
            BASE,
            "one\nA\nB\nC\nthree\n",
            &[(2, Change), (3, Change), (4, Change)],
        ),
        (
            "replacement shrinking 2 -> 1",
            BASE,
            "one\nX\n",
            &[(2, Change)],
        ),
        (
            "insertion beside a replacement, one hunk",
            ten,
            "1\nnew\n2\n3\nFOUR\n5\n6\n7\n8\n9\n10\n",
            &[(2, Add), (5, Change)],
        ),
        (
            "trailing-newline flip only",
            BASE,
            "one\ntwo\nthree",
            &[(3, Change)],
        ),
        ("empty tracked file gains content", "", "one\n", &[(1, Add)]),
        (
            "CRLF replacement",
            "one\r\ntwo\r\nthree\r\n",
            "one\r\nTWO\r\nthree\r\n",
            &[(2, Change)],
        ),
    ];
    for (name, old, new, want) in cases {
        assert_eq!(signs(old, new), *want, "{name}");
    }
}

#[test]
fn insertion_and_replacement_share_one_hunk() {
    // the table's mixed case only proves per-run classification if
    // libgit2 really merged both runs into one hunk
    let ten = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    let hunks = hunks_from_strings(
        ten,
        "1\nnew\n2\n3\nFOUR\n5\n6\n7\n8\n9\n10\n",
        Path::new("n"),
    )
    .unwrap();
    assert_eq!(hunks.len(), 1);
}
