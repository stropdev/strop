//! Ranking against a real tree's shape (0093 §6.1): the 97,362 tracked
//! paths of Linux 7.4-rc2 (`git ls-files` at 067c5c24cef2), names only.
//! The cases are the ones 0091 §4.1 found wrong on this tree.

use super::*;
use crate::{Item, Kind, Payload, Picker};
use std::io::Read;

fn corpus() -> Vec<Item> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/linux-7.4-rc2-paths.txt.gz"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    // the finder's catalog arrives in tree order
    let mut paths: Vec<&str> = text.lines().collect();
    paths.sort_by(|a, b| tree_order(a.as_bytes(), b.as_bytes()));
    paths
        .into_iter()
        .map(|path| Item {
            badge: None,
            text: path.to_owned(),
            payload: Payload::File(path.into()),
        })
        .collect()
}

fn finder(query: &str, priors: &[&str]) -> Picker {
    let mut picker = Picker::new(Kind::Files, corpus(), false);
    picker.priors = Priors::new(priors.iter().map(|path| path.to_string()));
    picker.paste(query);
    let ranking = rank(&picker.filter_request(), || false).unwrap().unwrap();
    assert!(picker.install_ranking(ranking));
    picker
}

fn top(picker: &Picker, rows: usize) -> Vec<&str> {
    picker
        .rows
        .iter()
        .take(rows)
        .map(|row| picker.items[row.item].text.as_str())
        .collect()
}

#[test]
fn a_typed_file_name_lists_the_files_with_that_name_shortest_path_first() {
    let picker = finder("sched.h", &[]);
    let rows = top(&picker, 12);
    assert!(
        rows.iter()
            .all(|path| path.rsplit('/').next() == Some("sched.h")),
        "{rows:?}"
    );
    assert_eq!(
        &rows[..2],
        ["kernel/sched/sched.h", "include/linux/sched.h"]
    );
    // the twelve exact names end there: the next row merely contains it
    assert_ne!(
        top(&picker, 13)[12].rsplit('/').next(),
        Some("sched.h"),
        "exactly twelve files are named sched.h"
    );
}

#[test]
fn the_file_you_opened_leads_its_equals() {
    let picker = finder("sched.h", &["include/linux/sched.h"]);
    assert_eq!(top(&picker, 1), ["include/linux/sched.h"]);
    // familiarity breaks ties; it never beats a better match
    let picker = finder("sched.h", &["kernel/sched/core.c"]);
    assert_eq!(top(&picker, 1), ["kernel/sched/sched.h"]);
}

#[test]
fn a_name_match_outranks_every_directory_match() {
    let picker = finder("sched", &[]);
    let rows = top(&picker, 20);
    assert_eq!(rows[0], "net/mptcp/sched.c");
    assert!(
        rows.iter()
            .all(|path| path.rsplit('/').next().unwrap().starts_with("sched")),
        "{rows:?}"
    );
}

#[test]
fn terms_are_anded_and_a_whole_name_beats_a_name_that_contains_it() {
    let picker = finder("core.c sched", &[]);
    assert_eq!(
        &top(&picker, 2),
        &["kernel/sched/core.c", "kernel/sched/core_sched.c"]
    );
    // the columns are both terms, left to right, each cell once
    let columns = picker.match_columns(&picker.rows[0]);
    let lit: String = columns
        .iter()
        .map(|&column| "kernel/sched/core.c".as_bytes()[column as usize] as char)
        .collect();
    assert_eq!(lit, "schedcore.c");
    // every row satisfies both terms somewhere in its path
    assert!(picker.rows.iter().all(|row| {
        let path = &picker.items[row.item].text;
        crate::fuzzy_score("core.c", path).is_some() && crate::fuzzy_score("sched", path).is_some()
    }));
}

#[test]
fn the_root_makefile_leads_two_thousand_makefiles() {
    let picker = finder("Makefile", &[]);
    assert_eq!(top(&picker, 1), ["Makefile"]);
    assert!(picker.rows.len() > 2_000);
}

#[test]
fn a_separator_in_a_term_addresses_the_path() {
    let picker = finder("sched/core", &[]);
    assert_eq!(top(&picker, 1), ["kernel/sched/core.c"]);
}

#[test]
fn the_empty_query_leads_with_what_was_reached_for_then_the_tree() {
    let picker = finder(
        "",
        &["kernel/sched/core.c", "mm/slub.c", "include/linux/sched.h"],
    );
    assert_eq!(picker.rows.len(), picker.items.len());
    assert_eq!(
        top(&picker, 5),
        [
            "kernel/sched/core.c",
            "mm/slub.c",
            "include/linux/sched.h",
            ".clang-format",
            ".clippy.toml",
        ]
    );
}
