//! 0056 AR04 crash/EOF/restart journeys: dirty documents, scratch drafts,
//! coherent cohorts, rename/delete/conflict source states, over-bound
//! cohorts, memory-only mode, remote consent and failed persistence.
//! Disk content stays unchanged until an explicit checked save.

use super::record::{DraftOrigin, Snapshot};
use super::{store, Editor};
use std::path::PathBuf;
use strop_core::Buffer;

struct Fixture {
    _dir: tempfile::TempDir,
    cwd: PathBuf,
    state: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    std::fs::create_dir(&cwd).unwrap();
    Fixture {
        state: dir.path().join("state"),
        cwd,
        _dir: dir,
    }
}

impl Fixture {
    fn editor(&self) -> Editor {
        let mut editor = Editor::new_in(Buffer::from_text(""), self.cwd.clone());
        editor.state_dir = Some(self.state.clone());
        editor
    }
    fn checkpoint(&self) -> PathBuf {
        store::checkpoint_path(&self.state, &self.cwd)
    }
    fn stored(&self) -> Option<store::StoredCohort> {
        store::load(&self.checkpoint()).unwrap()
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.cwd.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}

fn settle(editor: &mut Editor) {
    editor.wait_io().unwrap();
}

fn surface(editor: &Editor) -> String {
    editor.buf().text().to_string()
}

#[test]
fn dirty_document_checkpoints_and_restores_after_crash() {
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    // Checkpointing never marks text clean or advances its saved baseline.
    assert!(editor.buf().dirty);
    assert!(fixture.checkpoint().exists());
    // Crash: the editor drops without an orderly close.
    drop(editor);

    let mut restarted = fixture.editor();
    restarted.feed_text(":recover<cr>");
    settle(&mut restarted);
    let rows = surface(&restarted);
    assert!(rows.contains("a.txt"), "{rows}");
    assert!(rows.contains("[current]"), "{rows}");
    restarted.feed_text(":recover restore 1<cr>");
    settle(&mut restarted);
    assert_eq!(restarted.buf().text().to_string(), "lpha\n");
    let name = restarted.buf().name.clone().unwrap_or_default();
    assert!(name.starts_with("recovered: "), "{name}");
    assert!(
        restarted.buf().path.is_none(),
        "a checked draft has no target"
    );
    assert!(restarted.buf().dirty);
    // The disk was never touched by checkpoint or restore.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
}

#[test]
fn scratch_draft_checkpoints_and_restores_after_crash() {
    let fixture = fixture();
    let mut editor = fixture.editor();
    editor.feed_text("idraft words<esc>");
    settle(&mut editor);
    assert!(fixture.checkpoint().exists());
    drop(editor);

    let mut restarted = fixture.editor();
    restarted.feed_text(":recover<cr>");
    settle(&mut restarted);
    let rows = surface(&restarted);
    assert!(rows.contains("[scratch]"), "{rows}");
    restarted.feed_text(":recover restore 1<cr>");
    settle(&mut restarted);
    assert_eq!(restarted.buf().text().to_string(), "draft words");
}

#[test]
fn a_multi_document_change_captures_one_coherent_cohort() {
    let fixture = fixture();
    let a = fixture.write("a.txt", "one\n");
    let b = fixture.write("b.txt", "two\n");
    let mut editor = fixture.editor();
    let a_doc = editor.open_fixture(&a).unwrap();
    editor.open_fixture(&b).unwrap();
    editor.feed_text("x");
    editor.switch_to(a_doc);
    editor.feed_text("x");
    settle(&mut editor);

    let stored = fixture.stored().expect("a durable checkpoint");
    assert_eq!(stored.header.records.len(), 2, "{stored:?}");
    let cohort = stored.header.cohort;
    assert!(cohort > 0);
    let text_of = |name: &str| -> String {
        let (index, record) = stored
            .header
            .records
            .iter()
            .enumerate()
            .find(|(_, record)| record.origin.label().ends_with(name))
            .unwrap_or_else(|| panic!("a record for {name}"));
        assert!(matches!(record.snapshot, Snapshot::Captured { .. }));
        String::from_utf8(stored.texts[index].clone().unwrap()).unwrap()
    };
    assert_eq!(text_of("a.txt"), "ne\n");
    assert_eq!(text_of("b.txt"), "wo\n");
}

#[test]
fn a_live_rename_reports_the_renamed_state() {
    let fixture = fixture();
    let file = fixture.write("old.txt", "alpha\n");
    let mut editor = fixture.editor();
    let document = editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);

    let renamed = fixture.cwd.join("renamed.txt");
    std::fs::rename(&file, &renamed).unwrap();
    let stamp = std::fs::metadata(&renamed).and_then(|m| m.modified()).ok();
    let canonical = std::fs::canonicalize(&renamed).unwrap();
    editor
        .doc_mut(document)
        .buf
        .relocate_file_binding(canonical, stamp);

    editor.feed_text(":recover<cr>");
    settle(&mut editor);
    let rows = surface(&editor);
    assert!(rows.contains("[renamed]"), "{rows}");
    assert!(rows.contains("renamed.txt"), "{rows}");
}

#[test]
fn a_deleted_source_restores_without_recreating_it() {
    let fixture = fixture();
    let file = fixture.write("gone.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    drop(editor);

    std::fs::remove_file(&file).unwrap();
    let mut restarted = fixture.editor();
    restarted.feed_text(":recover<cr>");
    settle(&mut restarted);
    let rows = surface(&restarted);
    assert!(rows.contains("[missing]"), "{rows}");
    restarted.feed_text(":recover restore 1<cr>");
    settle(&mut restarted);
    assert_eq!(restarted.buf().text().to_string(), "lpha\n");
    assert!(restarted.buf().path.is_none());
    assert!(!file.exists(), "restore never recreates a deleted target");
}

#[test]
fn an_external_change_reports_conflict_and_never_overwrites() {
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    drop(editor);

    // Different length: the source observation mismatch is deterministic.
    std::fs::write(&file, "externally changed content\n").unwrap();
    let mut restarted = fixture.editor();
    restarted.feed_text(":recover<cr>");
    settle(&mut restarted);
    let rows = surface(&restarted);
    assert!(rows.contains("[conflict]"), "{rows}");
    restarted.feed_text(":recover restore 1<cr>");
    settle(&mut restarted);
    assert_eq!(restarted.buf().text().to_string(), "lpha\n");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "externally changed content\n",
        "restore never overwrites changed disk content"
    );
}

#[test]
fn an_over_bound_draft_is_reported_not_truncated() {
    let fixture = fixture();
    let small = fixture.write("small.txt", "one\n");
    let huge = fixture.write("huge.txt", &"x".repeat(17 * 1024 * 1024));
    let mut editor = fixture.editor();
    editor.open_fixture(&small).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    editor.open_fixture(&huge).unwrap();
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(huge.as_path()),
        "{}",
        editor.message
    );
    editor.feed_text("x");
    settle(&mut editor);

    let stored = fixture
        .stored()
        .expect("the fitting cohort still publishes");
    let huge_record = stored
        .header
        .records
        .iter()
        .find(|record| record.origin.label().ends_with("huge.txt"))
        .expect("the over-bound draft is reported in the cohort");
    assert!(
        matches!(huge_record.snapshot, Snapshot::OverBound { .. }),
        "{huge_record:?}"
    );
    let small_record = stored
        .header
        .records
        .iter()
        .find(|record| record.origin.label().ends_with("small.txt"))
        .expect("the fitting draft stays protected");
    assert!(matches!(small_record.snapshot, Snapshot::Captured { .. }));
    assert_eq!(editor.recovery_status().over_bound.len(), 1);

    editor.feed_text(":recover<cr>");
    settle(&mut editor);
    let rows = surface(&editor);
    assert!(rows.contains("[over-bound]"), "{rows}");
    let huge_index = stored
        .header
        .records
        .iter()
        .position(|record| record.origin.label().ends_with("huge.txt"))
        .unwrap()
        + 1;
    editor.feed_text(&format!(":recover restore {huge_index}<cr>"));
    settle(&mut editor);
    assert!(
        editor.message.contains("over the capture limit"),
        "{}",
        editor.message
    );
}

#[test]
fn memory_only_mode_persists_nothing_and_says_so() {
    let fixture = fixture();
    // No state directory: memory-only by construction.
    let mut editor = Editor::new_in(Buffer::from_text(""), fixture.cwd.clone());
    editor.feed_text("idraft<esc>");
    settle(&mut editor);
    assert!(editor.recovery_memory_only());
    editor.feed_text(":recover<cr>");
    settle(&mut editor);
    let rows = surface(&editor);
    assert!(rows.contains("memory-only"), "{rows}");
    assert!(rows.contains("NOT durable"), "{rows}");

    // A disabled session policy is memory-only even with a state directory.
    let mut disabled = fixture.editor();
    disabled.session_policy = crate::session::SessionPolicy::Disabled;
    disabled.feed_text("idraft<esc>");
    settle(&mut disabled);
    assert!(disabled.recovery_memory_only());
    assert!(!fixture.checkpoint().exists());
}

#[test]
fn remote_drafts_persist_only_with_explicit_session_consent() {
    let fixture = fixture();
    let mut editor = fixture.editor();
    let document = editor
        .docs
        .try_insert(crate::editor::test_support::remote::document(
            "remote bytes\n",
            strop_remote::ReadSelection::Full,
        ))
        .unwrap();
    editor.doc_mut(document).buf.dirty = true;
    editor.recovery_after_event();
    settle(&mut editor);
    assert!(
        !fixture.checkpoint().exists(),
        "no consent: remote drafts are never persisted"
    );

    editor.feed_text(":recover consent remote<cr>");
    settle(&mut editor);
    let stored = fixture.stored().expect("the consented cohort");
    let remote = stored
        .header
        .records
        .iter()
        .find(|record| matches!(record.origin, DraftOrigin::RemoteFile { .. }))
        .expect("consent persists the remote draft");
    assert!(remote.origin.label().contains("ssh://fixture"));

    editor.feed_text(":recover<cr>");
    settle(&mut editor);
    let rows = surface(&editor);
    assert!(rows.contains("[remote]"), "{rows}");
    editor.feed_text(":recover restore 1<cr>");
    settle(&mut editor);
    assert_eq!(editor.buf().text().to_string(), "remote bytes\n");
    assert!(
        editor.buf().path.is_none(),
        "restoring never re-grants remote authority"
    );
}

#[test]
fn a_confirmed_save_retires_only_the_checkpoint_it_supersedes() {
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    assert!(fixture
        .stored()
        .expect("checkpoint")
        .header
        .records
        .iter()
        .any(|record| record.origin.label().ends_with("a.txt")));

    editor.feed_text(":w<cr>");
    settle(&mut editor);
    assert!(!editor.buf().dirty);
    let stored = fixture.stored().expect("the retired cohort republishes");
    assert!(
        stored
            .header
            .records
            .iter()
            .all(|record| !record.origin.label().ends_with("a.txt")),
        "the confirmed save retired its checkpoint"
    );

    // Edits after that save still need recovery: the next capture is back.
    editor.feed_text("x");
    settle(&mut editor);
    assert!(fixture
        .stored()
        .expect("checkpoint")
        .header
        .records
        .iter()
        .any(|record| record.origin.label().ends_with("a.txt")));
}

#[test]
fn orderly_close_checkpoints_eligible_drafts() {
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    editor.finish_background_work();
    settle(&mut editor);
    assert!(editor.take_shutdown_error().is_none());
    assert!(fixture
        .stored()
        .expect("the orderly-close checkpoint")
        .header
        .records
        .iter()
        .any(|record| record.origin.label().ends_with("a.txt")));
}

#[test]
fn a_persistence_failure_is_visible_and_writes_nothing() {
    let fixture = fixture();
    // A regular file where the state directory must be: publication fails.
    let blocked = fixture.cwd.join("blocked-state");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = Editor::new_in(Buffer::from_text(""), fixture.cwd.clone());
    editor.state_dir = Some(blocked);
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    let status = editor.recovery_status();
    assert!(status.last_error.is_some(), "the failure reaches status");
    assert!(
        editor.message.contains("checkpoint failed") || status.last_error.is_some(),
        "{}",
        editor.message
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
    assert!(editor.buf().dirty, "recovery failure is not a save");
}

#[test]
fn a_deliberate_discard_drops_the_record() {
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.feed_text("x");
    settle(&mut editor);
    drop(editor);

    let mut restarted = fixture.editor();
    restarted.feed_text(":recover discard 1<cr>");
    settle(&mut restarted);
    let stored = fixture.stored().expect("the cohort survives the discard");
    assert!(
        stored
            .header
            .records
            .iter()
            .all(|record| !record.origin.label().ends_with("a.txt")),
        "the discarded record's bytes are gone"
    );
    let rows = surface(&restarted);
    assert!(
        rows.contains("no drafts") || !rows.contains("a.txt"),
        "{rows}"
    );
}

#[test]
fn a_suppressed_tape_never_launches_recovery_natively() {
    // 0057 VF15: capture/replay/recovery admission — under a fixture or
    // replay tape the native side is suppressed (tape.request answers
    // false): the request is RECORDED, but no worker is spawned and no
    // checkpoint bytes reach the disk. Replay and recovery never
    // acquire fresh process or filesystem authority.
    let fixture = fixture();
    let file = fixture.write("a.txt", "alpha\n");
    let mut editor = fixture.editor();
    editor.open_fixture(&file).unwrap();
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|operation, _| {
        panic!("no synchronous native query belongs to recovery: {operation}")
    }));
    editor.feed_text("x");
    editor.recovery_after_event();
    assert!(editor.buf().dirty, "the draft itself is untouched");
    let pending = editor
        .recovery
        .in_flight
        .expect("the publication was admitted onto the tape");
    assert!(
        !editor.worker_handles.contains_key(&pending),
        "suppressed: no native worker holds the request"
    );
    assert!(
        editor.recovery.queued.is_none(),
        "nothing pretends to be in progress"
    );
    assert!(
        !fixture.checkpoint().exists(),
        "no checkpoint bytes on disk"
    );
    assert!(
        editor.tape.fixture_nodes().iter().any(|node| matches!(
            node,
            strop_trace::replay::Node::Request { operation, .. }
                if operation == "io.recovery"
        )),
        "the request was recorded, never launched"
    );
}
