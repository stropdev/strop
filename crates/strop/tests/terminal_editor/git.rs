//! The working-tree journey on the real binary in a PTY (0091 "git"
//! acceptance, plan 0096 §4): edit two files, `Space g g`, stage one
//! file and one hunk of the other, commit, and `git log -1 --stat`
//! matches.
use super::*;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let done = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    String::from_utf8_lossy(&done.stdout).into_owned()
}

fn normal(screen: &str) -> bool {
    screen.contains(" NORMAL ")
}

#[test]
fn edit_two_files_stage_a_file_and_a_hunk_then_commit() {
    let directory = tempfile::tempdir().unwrap();
    // The editor starts beside the repository, not in it: its own state
    // directories never show up as untracked files, and the repository
    // is found from the buffer rather than from the working directory.
    let repo = directory.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "journey@example.invalid"]);
    git(&repo, &["config", "user.name", "Journey"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    let lines: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    std::fs::write(repo.join("a.txt"), lines).unwrap();
    std::fs::write(repo.join("b.txt"), "bee\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);

    let trace = directory.path().join("terminal.jsonl");
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(normal);

    // edit b.txt: one new line
    tui.send(b":e repo/b.txt\r");
    tui.until(|screen| numbered_line(screen, "bee"));
    tui.send(b"Ohoney");
    tui.until(|screen| numbered_line(screen, "honey") && screen.contains(" INSERT "));
    tui.send(b"\x1b");
    tui.until(normal);
    tui.send(b":w\r");
    tui.until(|screen| screen.contains("written"));

    // edit a.txt at both ends: two hunks
    tui.send(b":e repo/a.txt\r");
    tui.until(|screen| numbered_line(screen, "line 20"));
    tui.send(b"Otop");
    tui.until(|screen| numbered_line(screen, "top") && screen.contains(" INSERT "));
    tui.send(b"\x1b");
    tui.until(normal);
    tui.send(b"Gobottom");
    tui.until(|screen| numbered_line(screen, "bottom") && screen.contains(" INSERT "));
    tui.send(b"\x1b");
    tui.until(normal);
    tui.send(b":w\r");
    tui.until(|screen| screen.contains("written"));

    // the status buffer lists both; the cursor starts on the first file.
    // Both files are open, so their lock files sit beside them: those
    // are the editor's, and the status never lists them as changes.
    tui.send(b" gg");
    let listed = tui.until(|screen| {
        screen.contains("Unstaged (2)")
            && numbered_line(screen, "M a.txt")
            && numbered_line(screen, "M b.txt")
    });
    assert!(
        !listed.contains(".strop-lock") && !listed.contains("Untracked"),
        "{listed}"
    );

    // one hunk of a.txt: expand, step onto the first hunk, stage it
    tui.send(b"=");
    tui.until(|screen| numbered_line(screen, "+top") && numbered_line(screen, "+bottom"));
    tui.send(b"]c");
    tui.send(b"s");
    tui.until(|screen| screen.contains("hunk staged") && screen.contains("Staged (1)"));

    // all of b.txt: its row in the unstaged list, then s
    tui.send(b"G");
    tui.until(|screen| numbered_line(screen, "M b.txt"));
    tui.send(b"s");
    tui.until(|screen| screen.contains("staged b.txt") && screen.contains("Staged (2)"));

    // commit: the message is a buffer; :wq commits
    tui.send(b"cc");
    tui.until(|screen| screen.contains("COMMIT_EDITMSG") && screen.contains("# On branch main"));
    tui.send(b"ijourney: a file and a hunk");
    tui.until(|screen| numbered_line(screen, "journey: a file and a hunk"));
    tui.send(b"\x1b");
    tui.until(normal);
    tui.send(b":wq\r");
    tui.until(|screen| screen.contains("[main ") && screen.contains("journey: a file and a hunk"));

    let stat = git(&repo, &["log", "-1", "--stat", "--format=%s"]);
    assert!(stat.starts_with("journey: a file and a hunk\n"), "{stat}");
    assert!(stat.contains(" a.txt | 1 +\n"), "{stat}");
    assert!(stat.contains(" b.txt | 1 +\n"), "{stat}");
    assert!(
        stat.contains("2 files changed, 2 insertions(+)"),
        "exactly the staged file and the staged hunk: {stat}"
    );
    // the other hunk of a.txt was never staged: it is still there
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=no"]),
        " M a.txt\n"
    );
    assert!(git(&repo, &["diff"]).contains("+bottom"));
    assert!(git(&repo, &["show", "--format="]).contains("+top"));

    // back in the status buffer, which shows what is left
    tui.until(|screen| screen.contains("Unstaged (1)") && !screen.contains("Staged (2)"));
    tui.send(b":qa!\r");
    assert!(tui.wait_exit().success());
}
