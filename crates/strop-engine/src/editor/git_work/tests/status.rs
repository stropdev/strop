use super::*;
use strop_core::frontend_input::Key;

#[test]
fn status_lists_sections_and_lands_on_the_first_file() {
    let (dir, mut e) = status_fixture();
    assert_eq!(e.buf().name.as_deref(), Some("git status"));
    assert_eq!(
        text(&e),
        "Head: main\n\
         \n\
         Unstaged (2)\n\
         M notes.md\n\
         M src/lib.rs\n\
         \n\
         Untracked (1)\n\
         ? scratch.txt\n"
    );
    assert_eq!(cursor_line(&e), "M notes.md");
    assert!(e.buf().readonly);
    // a second press returns to the same buffer instead of stacking one
    let status = e.current();
    e.feed_text(":e src/lib.rs\r");
    settle_work(&mut e);
    key(&mut e, " gg");
    assert_eq!(e.current(), status);
    drop(dir);
}

#[test]
fn a_clean_tree_says_so() {
    let (_dir, mut e) = work_fixture();
    key(&mut e, " gg");
    assert_eq!(
        text(&e),
        "Head: main\n\nNothing to commit, working tree clean\n"
    );
    e.feed_text("s");
    assert_eq!(e.message, "no file or hunk here");
}

#[test]
fn s_stages_u_unstages_and_dash_toggles_the_file_under_the_cursor() {
    let (dir, mut e) = status_fixture();
    key(&mut e, "s");
    assert_eq!(e.message, "staged notes.md");
    assert_eq!(
        porcelain(dir.path()),
        "M  notes.md\n M src/lib.rs\n?? scratch.txt\n"
    );
    // the cursor stays in the unstaged list, on the file that took its place
    assert_eq!(cursor_line(&e), "M src/lib.rs");
    assert!(text(&e).contains("Staged (1)\nM notes.md\n\nUnstaged (1)\n"));

    goto(&mut e, "M notes.md");
    key(&mut e, "s");
    assert_eq!(e.message, "already staged");
    key(&mut e, "u");
    assert_eq!(e.message, "unstaged notes.md");
    assert!(porcelain(dir.path()).starts_with(" M notes.md\n"));

    goto(&mut e, "? scratch.txt");
    key(&mut e, "-");
    assert!(porcelain(dir.path()).contains("A  scratch.txt\n"));
    goto(&mut e, "A scratch.txt");
    key(&mut e, "-");
    assert!(porcelain(dir.path()).contains("?? scratch.txt\n"));
}

#[test]
fn a_stage_says_so_when_the_view_moved_while_it_ran() {
    // the index write refreshes the status buffer (a watcher hint, the
    // previous mutation landing) before this completion arrives: the
    // stage landed and the echo row still reports it
    let (dir, mut e) = status_fixture();
    e.feed_text("s");
    e.invalidate_git_view();
    e.refresh_status_buffers();
    settle_work(&mut e);
    assert_eq!(e.message, "staged notes.md");
    assert!(porcelain(dir.path()).starts_with("M  notes.md\n"));
}

#[test]
fn a_section_heading_acts_on_every_file_in_it() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "Unstaged (2)");
    key(&mut e, "s");
    assert_eq!(e.message, "staged 2 files");
    assert_eq!(
        porcelain(dir.path()),
        "M  notes.md\nM  src/lib.rs\n?? scratch.txt\n"
    );
    goto(&mut e, "Staged (2)");
    key(&mut e, "u");
    assert_eq!(e.message, "unstaged 2 files");
    assert_eq!(
        porcelain(dir.path()),
        " M notes.md\n M src/lib.rs\n?? scratch.txt\n"
    );
}

#[test]
fn equals_shows_the_diff_inline_and_s_stages_one_hunk() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "M src/lib.rs");
    key(&mut e, "=");
    let shown = text(&e);
    assert!(
        shown.contains("M src/lib.rs\n@@ -1,5 +1,5 @@\n line 1\n-line 2\n+line two\n line 3\n"),
        "{shown}"
    );
    assert!(shown.contains("-line 19\n+line nineteen\n"), "{shown}");
    assert_eq!(
        cursor_line(&e),
        "M src/lib.rs",
        "the cursor stays on the file"
    );

    // ]c walks hunk headers; s on a hunk stages only that hunk
    key(&mut e, "]c");
    assert_eq!(cursor_line(&e), "@@ -1,5 +1,5 @@");
    key(&mut e, "]c");
    assert!(cursor_line(&e).starts_with("@@ -16,5 +16,5 @@"));
    key(&mut e, "]c");
    assert_eq!(e.message, "no more hunks");
    key(&mut e, "[c");
    assert_eq!(cursor_line(&e), "@@ -1,5 +1,5 @@");
    key(&mut e, "j"); // a line inside the hunk works like its header
    key(&mut e, "s");
    assert_eq!(e.message, "hunk staged");
    assert!(porcelain(dir.path()).contains("MM src/lib.rs\n"));
    let staged = git_out(dir.path(), &["diff", "--cached", "--", "src/lib.rs"]);
    assert!(staged.contains("+line two") && !staged.contains("nineteen"));
    // the unstaged copy still shows the other hunk, and the cursor is on it
    assert!(cursor_line(&e).starts_with("@@ -16,5 +16,5 @@"));

    // = again, from any row of the diff, hides it
    key(&mut e, "=");
    assert_eq!(cursor_line(&e), "M src/lib.rs");
    assert!(!text(&e).contains("@@"));
}

#[test]
fn u_on_a_staged_hunk_unstages_only_that_hunk() {
    let (dir, mut e) = status_fixture();
    git(dir.path(), &["add", "src/lib.rs"]);
    key(&mut e, " gg");
    goto(&mut e, "M src/lib.rs");
    key(&mut e, "=");
    key(&mut e, "]c");
    key(&mut e, "u");
    assert_eq!(e.message, "hunk unstaged");
    let staged = git_out(dir.path(), &["diff", "--cached", "--", "src/lib.rs"]);
    assert!(!staged.contains("+line two") && staged.contains("+line nineteen"));
}

#[test]
fn x_discards_only_after_y_confirms() {
    let (dir, mut e) = status_fixture();
    key(&mut e, "X");
    assert_eq!(
        e.message,
        "discard changes to notes.md? y confirms, any other key cancels"
    );
    key(&mut e, "j");
    assert_eq!(e.message, "discard cancelled");
    assert_eq!(
        cursor_line(&e),
        "M notes.md",
        "the cancelling key is consumed"
    );
    assert!(porcelain(dir.path()).contains(" M notes.md"));

    key(&mut e, "X");
    key(&mut e, "y");
    assert_eq!(e.message, "discarded notes.md");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.md")).unwrap(),
        "# notes\n"
    );

    goto(&mut e, "? scratch.txt");
    key(&mut e, "X");
    assert!(e.message.starts_with("delete untracked scratch.txt?"));
    key(&mut e, "y");
    assert!(!dir.path().join("scratch.txt").exists());
    assert_eq!(porcelain(dir.path()), " M src/lib.rs\n");
}

#[test]
fn x_on_a_hunk_reverts_that_hunk_and_staged_rows_refuse() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "M src/lib.rs");
    key(&mut e, "=");
    key(&mut e, "]c");
    key(&mut e, "X");
    assert!(e.message.starts_with("discard this hunk of src/lib.rs?"));
    key(&mut e, "y");
    let file = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(file.contains("line 2\n") && file.contains("line nineteen\n"));

    goto(&mut e, "M notes.md");
    key(&mut e, "s");
    goto(&mut e, "M notes.md");
    key(&mut e, "X");
    assert_eq!(
        e.message,
        "discard works on unstaged changes — u unstages first"
    );
    key(&mut e, "y");
    assert!(porcelain(dir.path()).contains("M  notes.md"));
}

/// A buffer open on a discarded file shows the restored text.
#[test]
fn discarding_reloads_the_open_buffer() {
    let (dir, mut e) = work_fixture();
    let file = e.current();
    dirty(dir.path());
    e.feed_text(":e\r");
    settle_work(&mut e);
    assert_eq!(e.buf().line_text(1), "line two");
    key(&mut e, " gg");
    goto(&mut e, "M src/lib.rs");
    key(&mut e, "X");
    key(&mut e, "y");
    assert_eq!(e.doc(file).buf.line_text(1), "line 2");
}

#[test]
fn enter_opens_the_file_and_a_diff_row_opens_at_its_line() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "M src/lib.rs");
    key(&mut e, "=");
    goto(&mut e, "+line nineteen");
    e.feed(Key::Enter);
    settle_work(&mut e);
    assert_eq!(
        e.buf().path.as_deref(),
        Some(dir.path().join("src/lib.rs").as_path())
    );
    assert_eq!(e.buf().line_of(e.head()), 18, "the hunk row's own line");

    key(&mut e, " gg");
    goto(&mut e, "Unstaged");
    e.feed(Key::Enter);
    assert_eq!(e.message, "no file here");
}

#[test]
fn the_buffer_follows_the_tree_and_keeps_the_cursor() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "M src/lib.rs");
    // git ran somewhere else; the watcher (or focus) asks for a refresh
    git(dir.path(), &["add", "notes.md"]);
    std::fs::write(dir.path().join("another.txt"), "x\n").unwrap();
    e.refresh_status_buffers();
    settle_work(&mut e);
    assert_eq!(cursor_line(&e), "M src/lib.rs");
    assert!(text(&e).contains("Staged (1)\nM notes.md\n"));
    assert!(text(&e).contains("Untracked (2)\n? another.txt\n? scratch.txt\n"));
}

#[test]
fn motions_search_and_yank_work_like_any_buffer() {
    let (_dir, mut e) = status_fixture();
    key(&mut e, "gg/scratch\r");
    assert_eq!(cursor_line(&e), "? scratch.txt");
    key(&mut e, "yy");
    assert_eq!(e.register(None).text, "? scratch.txt\n");
    key(&mut e, "q");
    assert_ne!(e.buf().name.as_deref(), Some("git status"));
}

#[test]
fn a_remote_repository_refuses_in_one_sentence() {
    let (_dir, mut e) = work_fixture();
    e.git = Some(strop_git::GitContext {
        repo: strop_git::RepoTarget::Remote {
            endpoint: strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
            workdir: "/repo".into(),
        },
        head_sha: None,
        head_branch: None,
        remotes: Vec::new(),
    });
    let before = e.current();
    for (keys, what) in [(" gg", "status"), (" gc", "commit"), (" gb", "branches")] {
        e.feed_text(keys);
        assert_eq!(e.message, format!("git {what} needs a local repository"));
        assert_eq!(e.current(), before);
        assert!(e.picker.is_none());
    }
}

/// The editor keeps a lock file beside every open file. Those are its
/// own: never listed, never staged by a section heading, never counted
/// in the commit summary.
#[test]
fn the_editor_s_own_lock_files_are_not_changes() {
    let (dir, mut e) = work_fixture();
    dirty(dir.path());
    let lock = format!(".strop-lock-{}", "a".repeat(64));
    std::fs::write(dir.path().join(&lock), "").unwrap();
    std::fs::write(dir.path().join("src").join(&lock), "").unwrap();
    key(&mut e, " gg");
    assert!(!text(&e).contains(".strop-lock"), "{}", text(&e));
    assert!(text(&e).contains("Untracked (1)\n? scratch.txt\n"));
    goto(&mut e, "Untracked (1)");
    key(&mut e, "s");
    assert_eq!(e.message, "staged scratch.txt");
    key(&mut e, "cc");
    assert!(text(&e).contains("# Not in this commit: 2 unstaged, 0 untracked.\n"));
}

/// 0003 §5.7: every key this release adds renders in `?`.
#[test]
fn every_new_key_renders_in_help() {
    let (_dir, mut e) = work_fixture();
    e.feed_text(" ?");
    let help = text(&e);
    for row in [
        "status: changed files",
        "commit (:wq commits, :q! aborts)",
        "branches: switch / create",
        "blame gutter / card",
        "hunk: stage",
        "hunk: unstage",
        "hunk: discard",
        "status: stage / unstage / toggle the file, hunk or section",
        "status: show / hide the file's diff",
        "status: discard (y confirms)",
        "status: commit / amend",
        "status: open the file at the cursor",
        "status: next / prev hunk",
    ] {
        assert!(help.contains(row), "help is missing {row:?}:\n{help}");
    }
}

/// Saving a file in the editor is a change to the tree: an open status
/// buffer shows it without being asked.
#[test]
fn saving_a_file_updates_the_status_buffer() {
    let (_dir, mut e) = work_fixture();
    let file = e.current();
    key(&mut e, " gg");
    let status = e.current();
    assert!(text(&e).contains("working tree clean"));
    e.switch_to(file);
    e.feed_text("Onew first line");
    e.feed(Key::Esc);
    key(&mut e, ":w\r");
    // (the saved file now has a lock beside it: that is not a change)
    assert!(!e.doc(status).buf.text().to_string().contains("Untracked"));
    assert!(
        e.doc(status)
            .buf
            .text()
            .to_string()
            .contains("Unstaged (1)\nM src/lib.rs\n"),
        "{}",
        e.doc(status).buf.text()
    );
}

/// `s` on the diff of an untracked file adds the file itself.
#[test]
fn staging_an_untracked_file_from_its_diff_adds_the_file() {
    let (dir, mut e) = status_fixture();
    goto(&mut e, "? scratch.txt");
    key(&mut e, "=");
    goto(&mut e, "+todo");
    key(&mut e, "s");
    assert_eq!(e.message, "staged scratch.txt");
    assert!(porcelain(dir.path()).contains("A  scratch.txt\n"));
}
