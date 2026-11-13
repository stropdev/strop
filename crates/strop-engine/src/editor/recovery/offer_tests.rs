//! 0090 §3 recovery offer: opening a file with an earlier session's draft
//! asks first (vim `ATTENTION`), and a new session never destroys that
//! draft before the user answers.

use super::{store, Editor};
use crate::editor::InputOwner;
use std::path::{Path, PathBuf};
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
    /// A started session: the earlier session's drafts are read first.
    fn session(&self) -> Editor {
        let mut editor = Editor::new_in(Buffer::from_text(""), self.cwd.clone());
        editor.state_dir = Some(self.state.clone());
        editor.recovery_read_prior();
        editor.wait_io().unwrap();
        editor
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.cwd.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    fn stored_labels(&self) -> Vec<String> {
        store::load(&store::checkpoint_path(&self.state, &self.cwd))
            .unwrap()
            .map(|cohort| {
                cohort
                    .header
                    .records
                    .iter()
                    .map(|record| record.origin.label())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// A session that edited `a.txt` ("alpha" → "lpha") and crashed.
    fn crashed_draft(&self) -> PathBuf {
        let file = self.write("a.txt", "alpha\n");
        let mut editor = self.session();
        open(&mut editor, &file);
        editor.feed_text("x");
        editor.wait_io().unwrap();
        assert_eq!(self.stored_labels().len(), 1);
        drop(editor);
        file
    }
}

fn open(editor: &mut Editor, file: &Path) {
    editor.feed_text(&format!(":e {}\r", file.display()));
    editor.wait_io().unwrap();
}

#[test]
fn opening_a_file_with_a_draft_offers_it_and_recover_is_one_undo() {
    let fixture = fixture();
    let file = fixture.crashed_draft();
    let mut editor = fixture.session();
    open(&mut editor, &file);
    assert_eq!(editor.input_owner(), InputOwner::RecoveryOffer);
    let offer = editor.recovery_offer().unwrap();
    assert!(offer.path.ends_with("a.txt"), "{offer:?}");
    assert!(!offer.source_changed, "{offer:?}");
    // keys other than the four answers never reach the file
    editor.feed_text("x");
    assert_eq!(editor.buf().text().to_string(), "alpha\n");
    assert!(
        editor.message().contains("r recover"),
        "{}",
        editor.message()
    );
    editor.feed_text("r");
    assert_eq!(editor.input_owner(), InputOwner::Document);
    assert_eq!(editor.buf().text().to_string(), "lpha\n");
    assert!(editor.buf().is_dirty());
    assert_eq!(editor.buf().path.as_deref(), Some(file.as_path()));
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "alpha\n");
    assert!(!editor.buf().is_dirty(), "u returns to the file on disk");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
}

#[test]
fn a_new_session_keeps_the_earlier_draft_until_answered() {
    let fixture = fixture();
    let a = fixture.crashed_draft();
    let b = fixture.write("b.txt", "beta\n");
    let mut editor = fixture.session();
    // editing another file publishes this session's checkpoint
    open(&mut editor, &b);
    editor.feed_text("x");
    editor.wait_io().unwrap();
    let labels = fixture.stored_labels();
    assert!(
        labels.iter().any(|label| label.ends_with("a.txt")),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label.ends_with("b.txt")),
        "{labels:?}"
    );
    // `e` keeps the draft: edit the disk text, the draft stays listed
    editor.feed_text(":w\r");
    editor.wait_io().unwrap();
    open(&mut editor, &a);
    assert_eq!(editor.input_owner(), InputOwner::RecoveryOffer);
    editor.feed_text("e");
    assert_eq!(editor.input_owner(), InputOwner::Document);
    assert_eq!(editor.buf().text().to_string(), "alpha\n");
    assert!(!editor.buf().readonly);
    editor.wait_io().unwrap();
    assert!(fixture
        .stored_labels()
        .iter()
        .any(|label| label.ends_with("a.txt")));
}

#[test]
fn delete_drops_the_draft_for_good() {
    let fixture = fixture();
    let file = fixture.crashed_draft();
    let mut editor = fixture.session();
    open(&mut editor, &file);
    editor.feed_text("d");
    editor.wait_io().unwrap();
    assert_eq!(editor.input_owner(), InputOwner::Document);
    assert!(fixture.stored_labels().is_empty());
    drop(editor);
    let mut later = fixture.session();
    open(&mut later, &file);
    assert_eq!(
        later.input_owner(),
        InputOwner::Document,
        "nothing to offer"
    );
}

#[test]
fn read_only_keeps_the_draft_and_a_changed_file_is_flagged() {
    let fixture = fixture();
    let file = fixture.crashed_draft();
    std::fs::write(&file, "changed elsewhere\n").unwrap();
    let mut editor = fixture.session();
    open(&mut editor, &file);
    assert!(editor.recovery_offer().unwrap().source_changed);
    editor.feed_text("o");
    assert!(editor.buf().readonly);
    assert_eq!(editor.buf().text().to_string(), "changed elsewhere\n");
    editor.wait_io().unwrap();
    assert_eq!(fixture.stored_labels().len(), 1, "the draft is kept");
}

/// `:q` before the startup read lands: the read finishes against an
/// editor with no documents and no panes, and must not reach for a
/// current file (CI caught this as a panic on exit).
#[test]
fn the_startup_read_can_land_after_the_session_closed() {
    let fixture = fixture();
    let file = fixture.crashed_draft();
    let mut editor = Editor::new_in(Buffer::from_text(""), fixture.cwd.clone());
    editor.state_dir = Some(fixture.state.clone());
    editor.recovery_read_prior();
    editor.feed_text(":q\r");
    assert!(editor.should_quit());
    editor.wait_io().unwrap();
    let labels = fixture.stored_labels();
    assert_eq!(
        labels,
        vec![file.display().to_string()],
        "the draft survives"
    );
}
