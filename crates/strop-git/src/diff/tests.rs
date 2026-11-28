//! Gutter signs per change run (#240): what a diff between two texts
//! puts on which new-side line, before any row clamping.

use super::Sign::{self, Add, Change, DeleteAfter};
use crate::repo::hunks_from_strings;
use std::path::Path;

fn signs(old: &str, new: &str) -> Vec<(usize, Sign)> {
    hunks_from_strings(old, new, Path::new("note"))
        .unwrap()
        .iter()
        .flat_map(|hunk| {
            let mut regions = Vec::new();
            hunk.regions(&mut regions);
            regions
        })
        .flat_map(|region| Sign::of_region(&region).collect::<Vec<_>>())
        // 1-based, as the gutter keys them
        .map(|(line, sign)| (line + 1, sign))
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
        ("delete everything", BASE, "", &[(1, DeleteAfter)]),
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

/// A deterministic xorshift stream: the property test stays hermetic.
struct Stream(u64);

impl Stream {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

/// #287/#288: over the production diff, the kernel's regions describe the
/// two texts. Every region list is coherent, every kept new line maps to an
/// old line with the same text and back, and kept plus changed lines count
/// each side exactly.
#[test]
fn regions_describe_the_two_texts() {
    use strop_kernel::lines::{coherent, new_line, old_line};
    let mut stream = Stream(0x9e37_79b9_7f4a_7c15);
    for case in 0..400 {
        let old: Vec<&str> = (0..stream.below(16))
            .map(|_| ["a", "b", "c", "d"][stream.below(4)])
            .collect();
        let mut new = old.clone();
        for _ in 0..stream.below(5) {
            let at = stream.below(new.len() + 1);
            match stream.below(3) {
                0 => new.insert(at, ["a", "b", "x", "y"][stream.below(4)]),
                1 if at < new.len() => {
                    new.remove(at);
                }
                _ if at < new.len() => new[at] = "z",
                _ => {}
            }
        }
        let text = |lines: &[&str]| {
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        };
        let hunks = hunks_from_strings(&text(&old), &text(&new), Path::new("p")).unwrap();
        let mut regions = Vec::new();
        for hunk in &hunks {
            hunk.regions(&mut regions);
        }
        assert!(coherent(&regions), "case {case}: {regions:?}");
        let mut kept = 0;
        for (n, line) in new.iter().enumerate() {
            if let Some(o) = old_line(n, &regions) {
                assert_eq!(old[o], *line, "case {case}: new {n} -> old {o}");
                assert_eq!(
                    new_line(o, &regions),
                    Some(n),
                    "case {case}: back from old {o}"
                );
                kept += 1;
            }
        }
        let added: usize = regions.iter().map(|r| r.new_len).sum();
        let deleted: usize = regions.iter().map(|r| r.old_len).sum();
        assert_eq!(kept + added, new.len(), "case {case}");
        assert_eq!(kept + deleted, old.len(), "case {case}");
    }
}
