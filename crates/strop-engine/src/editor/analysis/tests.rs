//! Analysis ownership: interim frames, failures, syntax identity, and
//! the worker seam under interleavings (0066, 0111).
use super::*;
use std::path::PathBuf;
use strop_core::worker::WorkerId;
use strop_core::Buffer;
use strop_syntax::Span;

/// 0066 S1: after an edit, the readonly paint query serves the
/// accepted colors at journal-mapped coordinates — not a blank
/// pane — while the fresh exact analysis is still in flight. The
/// worker completion is deliberately left unpumped.
#[test]
fn an_edit_serves_edit_mapped_spans_instead_of_blanking() {
    let mut e = Editor::new(Buffer::from_text("fn main() {}\nfn other() {}\n"));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
    let doc = e.current();
    let warm = e.analysis_fixture().spans.clone();
    assert!(!warm.is_empty(), "the rust grammar analyzes this source");
    // one space inserted at byte zero: every warm span shifts by one
    e.feed_text("i <esc>");
    let served = e
        .document_analysis_cached(doc, 0, e.buf().len_bytes(), 0, 100)
        .expect("the interim frame serves the accepted colors");
    let expected: Vec<Span> = warm
        .iter()
        .map(|span| Span {
            start: span.start + 1,
            end: span.end + 1,
            ..*span
        })
        .collect();
    assert_eq!(served.spans, expected, "spans follow the edit");
    // preparation admits the fresh window; it is still in flight
    let _ = e.document_analysis(doc, 0, e.buf().len_bytes(), 0, 100);
    assert!(
        e.analysis_pending_probe(),
        "this is the interim frame, not the fresh result"
    );
    e.wait_analysis(); // drain the cancelled pre-edit request
    let exact = e.analysis_fixture_width(100);
    assert_eq!(exact.spans, expected, "the settled parse agrees");
}

/// 0066: a viewport that moved without an edit keeps its covered
/// overlap colored while the new window's analysis is in flight —
/// scrolling must not blank the pane either.
#[test]
fn a_scrolled_viewport_serves_the_covered_overlap() {
    let mut e = Editor::new(Buffer::from_text(
        "fn one() {}\nfn two() {}\nfn three() {}\n",
    ));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
    let doc = e.current();
    let warm = e.analysis_fixture_width(80).spans.clone();
    let len = e.buf().len_bytes();
    let line1 = e.buf().line_end(0);
    let served = e
        .document_analysis_cached(doc, line1, len, 0, 100)
        .expect("the covered overlap stays colored");
    let expected: Vec<Span> = warm
        .iter()
        .filter(|span| span.start.max(line1) < span.end.min(len))
        .map(|span| Span {
            start: span.start.max(line1),
            end: span.end.min(len),
            ..*span
        })
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(served.spans, expected);
}

/// 0066 §3C: a real analysis failure invalidates the window's
/// provisional coverage instead of leaving it looking settled.
#[test]
fn a_failed_window_loses_its_provisional_coverage() {
    let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
    let doc = e.current();
    e.analysis_fixture();
    e.feed_text("i <esc>");
    let key = e
        .analysis_key(doc, 0, e.buf().len_bytes(), 0, 100)
        .expect("syntax surface");
    let target = key.target.clone();
    e.analysis.pending.insert(
        target.clone(),
        Pending {
            ticket: Ticket {
                request: WorkerId::new(1),
                key: key.clone(),
            },
            cancel: Arc::new(AtomicBool::new(false)),
        },
    );
    e.handle_analysis(AnalysisEvent::Completed(Box::new(Completion {
        ticket: Ticket {
            request: WorkerId::new(1),
            key,
        },
        outcome: Outcome::failed(strop_core::worker::FailureKind::Io, "boom".to_string()),
    })));
    assert!(
        e.document_analysis_cached(doc, 0, e.buf().len_bytes(), 0, 100)
            .is_none(),
        "failed coverage must not keep serving projected colors"
    );
}

/// Deliver a completion for the pending ticket of `target`, as if the
/// worker had produced it.
fn complete_pending(e: &mut Editor, target: &AnalysisTarget, outcome: Outcome<FrameAnalysis>) {
    let ticket = e
        .analysis
        .pending
        .get(target)
        .expect("a pending request")
        .ticket
        .clone();
    e.handle_analysis(AnalysisEvent::Completed(Box::new(Completion {
        ticket,
        outcome,
    })));
}

/// 0111 decision 1: a failed window is decided. Asking again for the
/// same key neither requests new work nor leaves the pane loading.
#[test]
fn a_failed_analysis_is_remembered_not_requested_every_frame() {
    let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
    let doc = e.current();
    let len = e.buf().len_bytes();
    let _ = e.document_analysis(doc, 0, len, 0, 80);
    let target = AnalysisTarget::Document(doc);
    complete_pending(
        &mut e,
        &target,
        Outcome::failed(strop_core::worker::FailureKind::Io, "boom".to_string()),
    );
    e.wait_analysis(); // the real worker's answer for the same ticket is dropped
    let admitted = e.admissions.analysis;
    for _ in 0..3 {
        let _ = e.document_analysis(doc, 0, len, 0, 80);
    }
    assert_eq!(e.admissions.analysis, admitted, "no re-request per frame");
    assert!(!e.analysis.pending.contains_key(&target));
    assert!(!e.window_analysis_pending_for(doc, 0, len, 0, 80));
    // a new key (an edit) asks again
    e.feed_text("i <esc>");
    let _ = e.document_analysis(doc, 0, e.buf().len_bytes(), 0, 80);
    assert_eq!(e.admissions.analysis, admitted + 1);
}

/// 0111 decision 7: an extensionless file's shebang is read when the
/// text meets the disk. Typed, it takes effect at `:w`.
#[test]
fn a_typed_shebang_takes_effect_at_write() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("script");
    std::fs::write(&path, "echo \"hi\"\nif true; then exit 1; fi\n").expect("fixture");
    let mut e = Editor::new(Buffer::from_text(""));
    let doc = e.open_fixture(&path).expect("open");
    assert_eq!(e.doc(doc).syntax_language(), None);
    e.feed_text("ggO#!/bin/bash<esc>");
    assert_eq!(
        e.doc(doc).syntax_language(),
        None,
        "typing alone re-detects nothing"
    );
    e.feed_text(":w<cr>");
    e.wait_io().expect("save");
    assert_eq!(
        e.doc(doc).syntax_language(),
        Some(strop_syntax::languages::LanguageId::Bash)
    );
    let spans = e.analysis_fixture().spans.clone();
    assert!(!spans.is_empty(), "bash colours land after the write");
    // the shebang deleted and written: back to no language
    e.feed_text("ggdd:w<cr>");
    e.wait_io().expect("save");
    assert_eq!(e.doc(doc).syntax_language(), None);
}

/// 0111 decision 8: retained spans belong to the syntax identity that
/// produced them; another language's interim frame never borrows them.
#[test]
fn the_interim_frame_never_serves_another_languages_spans() {
    let mut e = Editor::new(Buffer::from_text("fn one() {}\nfn two() {}\n"));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
    let doc = e.current();
    assert!(!e.analysis_fixture_width(80).spans.is_empty());
    let len = e.buf().len_bytes();
    let line1 = e.buf().line_end(0);
    assert!(e
        .document_analysis_cached(doc, line1, len, 0, 100)
        .is_some_and(|frame| !frame.spans.is_empty()));
    e.buf_mut().path = Some(PathBuf::from("/workspace/a.py"));
    assert!(e
        .document_analysis_cached(doc, line1, len, 0, 100)
        .is_none_or(|frame| frame.spans.is_empty()));
}

mod interleaving;
