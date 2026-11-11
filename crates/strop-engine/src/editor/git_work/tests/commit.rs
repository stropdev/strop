use super::*;
use strop_core::frontend_input::Key;

/// Stage `notes.md` from the status buffer and open the commit message.
fn commit_fixture() -> (tempfile::TempDir, Editor) {
    let (dir, mut e) = status_fixture();
    key(&mut e, "s");
    key(&mut e, "cc");
    (dir, e)
}

fn write_message(e: &mut Editor, message: &str) {
    e.feed_text("i");
    e.paste_bracketed(message);
    e.feed(Key::Esc);
}

#[test]
fn cc_opens_the_message_file_with_the_staged_summary() {
    let (dir, e) = commit_fixture();
    assert_eq!(
        e.buf().path.as_deref(),
        Some(dir.path().join(".git/COMMIT_EDITMSG").as_path())
    );
    assert!(!e.buf().readonly);
    let template = text(&e);
    assert!(template.starts_with("\n\n# Write the message above"));
    assert!(template.contains("# On branch main\n"));
    assert!(template.contains("# Changes to be committed:\n#\tM notes.md\n"));
    assert!(template.contains("# Not in this commit: 1 unstaged, 1 untracked.\n"));
    assert_eq!(e.head(), 0, "the cursor is where the message goes");
}

#[test]
fn wq_commits_what_was_staged_and_nothing_else() {
    let (dir, mut e) = commit_fixture();
    write_message(&mut e, "notes: say more");
    key(&mut e, ":wq\r");
    assert_eq!(subject(dir.path()), "notes: say more");
    assert!(
        e.message.starts_with("[main ") && e.message.ends_with("notes: say more"),
        "git's own summary line: {}",
        e.message
    );
    assert_eq!(
        git_out(dir.path(), &["log", "-1", "--format=", "--name-only"]),
        "notes.md"
    );
    assert_eq!(porcelain(dir.path()), " M src/lib.rs\n?? scratch.txt\n");
    // back in the status buffer, which already shows the new tree
    assert_eq!(e.buf().name.as_deref(), Some("git status"));
    assert!(!text(&e).contains("Staged"));
    // the comment lines never reach the message
    assert_eq!(
        git_out(dir.path(), &["log", "-1", "--format=%B"]),
        "notes: say more"
    );
}

#[test]
fn w_then_q_commits_too() {
    let (dir, mut e) = commit_fixture();
    write_message(&mut e, "two steps");
    key(&mut e, ":w\r");
    assert_eq!(subject(dir.path()), "base", "writing alone commits nothing");
    key(&mut e, ":q\r");
    assert_eq!(subject(dir.path()), "two steps");
}

#[test]
fn q_bang_aborts() {
    let (dir, mut e) = commit_fixture();
    write_message(&mut e, "never mind");
    key(&mut e, ":q!\r");
    assert_eq!(e.message, "commit aborted");
    assert_eq!(subject(dir.path()), "base");
    assert!(
        porcelain(dir.path()).contains("M  notes.md"),
        "still staged"
    );
}

#[test]
fn an_empty_message_is_git_s_refusal() {
    let (dir, mut e) = commit_fixture();
    key(&mut e, ":wq\r");
    assert!(
        e.message.contains("empty commit message"),
        "git's words: {}",
        e.message
    );
    assert_eq!(subject(dir.path()), "base");
}

#[test]
fn nothing_staged_refuses_before_a_buffer_opens() {
    let (_dir, mut e) = status_fixture();
    let status = e.current();
    key(&mut e, "cc");
    assert_eq!(e.message, "nothing staged — s stages a file in Space g g");
    assert_eq!(e.current(), status);
}

#[test]
fn space_g_c_commits_from_a_file_buffer() {
    let (dir, mut e) = work_fixture();
    dirty(dir.path());
    git(dir.path(), &["add", "scratch.txt"]);
    let file = e.current();
    key(&mut e, " gc");
    assert!(text(&e).contains("#\tA scratch.txt\n"));
    write_message(&mut e, "add scratch");
    key(&mut e, ":wq\r");
    assert_eq!(subject(dir.path()), "add scratch");
    assert_eq!(e.current(), file, "closing returns to the file");
}

#[test]
fn ca_amends_starting_from_the_last_message() {
    let (dir, mut e) = status_fixture();
    key(&mut e, "s");
    key(&mut e, "ca");
    assert!(text(&e).starts_with("base\n\n# "));
    assert!(text(&e).contains("# Amending the last commit.\n"));
    e.feed_text("cc");
    e.paste_bracketed("base, with notes");
    e.feed(Key::Esc);
    key(&mut e, ":wq\r");
    assert_eq!(subject(dir.path()), "base, with notes");
    assert_eq!(
        git_out(dir.path(), &["rev-list", "--count", "HEAD"]),
        "1",
        "amended, not added"
    );
    assert!(git_out(dir.path(), &["show", "--stat", "--format="]).contains("notes.md"));
}

/// Hooks are git's: a failing pre-commit stops the commit and its
/// output is what the echo row shows.
#[test]
fn a_failing_hook_stops_the_commit_with_its_own_words() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut e) = commit_fixture();
    let hook = dir.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'lint: trailing space' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    write_message(&mut e, "blocked");
    key(&mut e, ":wq\r");
    assert_eq!(e.message, "lint: trailing space");
    assert_eq!(subject(dir.path()), "base");
    assert!(porcelain(dir.path()).contains("M  notes.md"));
}

/// In a split, `:wq` still commits: closing the message is the trigger,
/// not closing the last pane.
#[test]
fn wq_commits_from_a_split() {
    let (dir, mut e) = commit_fixture();
    e.feed_text(":vs\r");
    write_message(&mut e, "from a split");
    key(&mut e, ":wq\r");
    assert_eq!(subject(dir.path()), "from a split");
}

fn branch_rows(e: &Editor) -> Vec<String> {
    let glue = e.picker.as_ref().expect("the branch picker is open");
    assert_eq!(glue.picker.kind, strop_picker::Kind::Branches);
    glue.picker
        .items
        .iter()
        .map(|item| item.text.clone())
        .collect()
}

fn branch(root: &Path) -> String {
    git_out(root, &["branch", "--show-current"])
}

fn accept(e: &mut Editor) {
    e.wait_picker();
    e.feed(Key::Enter);
    e.wait_picker();
    settle_work(e);
}

#[test]
fn space_g_b_lists_branches_and_enter_switches() {
    let (dir, mut e) = work_fixture();
    git(dir.path(), &["branch", "topic"]);
    key(&mut e, " gb");
    assert_eq!(
        branch_rows(&e),
        [
            "main — current",
            "topic",
            "create a branch from the typed name"
        ]
    );
    e.paste_bracketed("top");
    accept(&mut e);
    assert_eq!(branch(dir.path()), "topic");
    assert_eq!(e.message, "Switched to branch 'topic'");
    assert_eq!(
        e.git.as_ref().unwrap().head_branch.as_deref(),
        Some("topic"),
        "the modeline's branch follows"
    );
}

#[test]
fn a_switch_that_would_lose_work_is_refused_in_git_s_words() {
    let (dir, mut e) = work_fixture();
    git(dir.path(), &["checkout", "-qb", "topic"]);
    std::fs::write(dir.path().join("notes.md"), "# topic notes\n").unwrap();
    git(dir.path(), &["commit", "-qam", "topic notes"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("notes.md"), "# local edit\n").unwrap();
    key(&mut e, " gb");
    e.paste_bracketed("topic");
    accept(&mut e);
    assert_eq!(branch(dir.path()), "main");
    assert!(
        e.message.contains("would be overwritten by checkout") && e.message.contains("notes.md"),
        "{}",
        e.message
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.md")).unwrap(),
        "# local edit\n"
    );
}

#[test]
fn the_typed_name_creates_a_branch() {
    let (dir, mut e) = work_fixture();
    key(&mut e, " gb");
    e.paste_bracketed("feature/status");
    accept(&mut e);
    assert_eq!(branch(dir.path()), "feature/status");
    assert_eq!(e.message, "Switched to a new branch 'feature/status'");

    // git validates the name; an empty one never reaches it
    key(&mut e, " gb");
    e.paste_bracketed("bad..name");
    accept(&mut e);
    assert!(
        e.message.contains("not a valid branch name"),
        "{}",
        e.message
    );
    assert_eq!(branch(dir.path()), "feature/status");
}

/// Switching changes the files under open buffers: a clean buffer
/// shows the new branch's text.
#[test]
fn switching_reloads_clean_buffers() {
    let (dir, mut e) = work_fixture();
    git(dir.path(), &["checkout", "-qb", "topic"]);
    std::fs::write(dir.path().join("src/lib.rs"), "topic text\n").unwrap();
    git(dir.path(), &["commit", "-qam", "topic text"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    assert_eq!(e.buf().line_text(0), "line 1");
    key(&mut e, " gb");
    e.paste_bracketed("topic");
    accept(&mut e);
    assert_eq!(e.buf().line_text(0), "topic text");
}
