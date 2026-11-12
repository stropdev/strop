//! A history surface fills in pages (0101 §2.2): the first replaces
//! `loading log…`, later ones are appended, nothing above the cursor
//! moves, and a page for a surface that is gone lands nowhere.

use super::{commit_row, log_done};
use crate::editor::git_memory::LogRequest;
use crate::editor::test_support::git::{fixture, pump};
use crate::editor::{Editor, GitJob, Surface};
use strop_core::id::DocumentId;
use strop_core::worker::{Completion, Outcome, Ticket};
use strop_git::log::LogRow;

/// A log surface whose request is ours to answer: the real job's
/// results are refused.
fn surface(e: &mut Editor) -> (DocumentId, Ticket<crate::editor::git_memory::types::LogKey>) {
    e.open_log(false);
    let doc = e.current();
    let real = e.log_requests[&doc].ticket.clone();
    let owner = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: real.key.clone(),
    };
    e.log_requests.insert(
        doc,
        LogRequest {
            ticket: owner.clone(),
            expected: real.key.revision,
            published: false,
        },
    );
    (doc, owner)
}

fn page(
    e: &mut Editor,
    ticket: &Ticket<crate::editor::git_memory::types::LogKey>,
    rows: Vec<LogRow>,
) {
    e.handle_git_job(GitJob::LogPage(ticket.clone(), rows));
}

fn rows(e: &Editor, doc: DocumentId) -> usize {
    match e.doc(doc).surface_payload() {
        Some(Surface::CommitLog { rows, .. }) => rows.len(),
        _ => panic!("a log surface"),
    }
}

#[test]
fn pages_append_and_nothing_above_the_cursor_moves() {
    let (_d, mut e) = fixture();
    let (doc, ticket) = surface(&mut e);
    assert_eq!(e.buf().text().to_string(), "loading log…");
    page(
        &mut e,
        &ticket,
        vec![commit_row("aaaa", "one"), commit_row("bbbb", "two")],
    );
    let first = e.buf().text().to_string();
    assert_eq!(first, "* aaaa t · now · one\n* bbbb t · now · two\n");
    // the cursor on the second row, the view where it is
    e.feed_text("j");
    let head = e.head();
    page(
        &mut e,
        &ticket,
        vec![LogRow::new("|\\".into(), None), commit_row("cccc", "three")],
    );
    assert!(e.buf().text().to_string().starts_with(&first), "appended");
    assert_eq!(e.head(), head, "the cursor stays");
    assert_eq!(rows(&e, doc), 4);
    // the end carries the last rows
    e.handle_git_job(GitJob::Log(Completion {
        ticket,
        outcome: Outcome::Success(log_done(vec![commit_row("dddd", "four")])),
    }));
    assert_eq!(rows(&e, doc), 5);
    assert_eq!(e.buf().len_lines(), 6, "five rows and the final newline");
    assert!(e
        .buf()
        .text()
        .to_string()
        .ends_with("* dddd t · now · four\n"));
    assert!(!e.log_requests.contains_key(&doc));
}

/// An empty log is a completed empty list — the loading text goes.
#[test]
fn an_empty_history_replaces_the_loading_text() {
    let (_d, mut e) = fixture();
    let (_, ticket) = surface(&mut e);
    e.handle_git_job(GitJob::Log(Completion {
        ticket,
        outcome: Outcome::Success(log_done(Vec::new())),
    }));
    assert_eq!(e.buf().text().to_string(), "\n");
}

/// The blame dive's focus is found in whichever page carries it.
#[test]
fn the_focus_row_is_found_in_a_later_page() {
    let (_d, mut e) = fixture();
    let (doc, ticket) = surface(&mut e);
    if let Some(Surface::CommitLog { focus, .. }) = e.doc_mut(doc).surface_payload_mut() {
        *focus = Some("cccc".into());
    }
    page(
        &mut e,
        &ticket,
        vec![commit_row("aaaa", "one"), commit_row("bbbb", "two")],
    );
    assert_eq!(e.buf().line_of(e.head()), 0);
    page(&mut e, &ticket, vec![commit_row("cccc", "three")]);
    assert_eq!(e.buf().line_of(e.head()), 2, "landed on the focus");
}

/// A page for a closed surface, or for a request that was superseded,
/// lands nowhere; the live path still delivers.
#[test]
fn stale_pages_are_dropped() {
    let (_d, mut e) = fixture();
    let (doc, ticket) = surface(&mut e);
    page(&mut e, &ticket, vec![commit_row("aaaa", "one")]);
    e.feed_text("q");
    page(&mut e, &ticket, vec![LogRow::new("POISON".into(), None)]);
    for (_, document) in e.docs.iter() {
        assert!(!document.buf.text().to_string().contains("POISON"));
    }
    assert!(e.docs.get(doc).is_none());
    // superseded: a newer request owns the surface
    let (doc, old) = surface(&mut e);
    let (_, newer) = {
        let real = e.log_requests[&doc].clone();
        let newer = Ticket {
            request: e.worker_ids.allocate().unwrap(),
            key: real.ticket.key.clone(),
        };
        e.log_requests.insert(
            doc,
            LogRequest {
                ticket: newer.clone(),
                ..real
            },
        );
        (doc, newer)
    };
    page(&mut e, &old, vec![LogRow::new("POISON".into(), None)]);
    assert!(!e.buf().text().to_string().contains("POISON"));
    page(&mut e, &newer, vec![commit_row("aaaa", "live")]);
    assert!(e.buf().text().to_string().contains("live"));
}

/// A read that fails after pages landed leaves its failure, and no rows
/// that the text no longer shows.
#[test]
fn a_failure_after_pages_takes_their_rows_with_it() {
    let (_d, mut e) = fixture();
    let (doc, ticket) = surface(&mut e);
    page(&mut e, &ticket, vec![commit_row("aaaa", "one")]);
    e.handle_git_job(GitJob::Log(Completion {
        ticket,
        outcome: Outcome::failed(strop_core::worker::FailureKind::Exit, "boom"),
    }));
    assert_eq!(e.buf().text().to_string(), "git log failed: boom\n");
    assert_eq!(rows(&e, doc), 0);
}

/// The real job: every page lands, and the log reads as git printed it.
#[test]
fn the_real_log_fills_the_surface() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    let text = e.buf().text().to_string();
    assert!(text.contains("add b") && text.contains("first"), "{text}");
    assert_eq!(rows(&e, e.current()), text.lines().count());
}
