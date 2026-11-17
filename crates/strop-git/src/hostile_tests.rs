//! A repository's own config must not run programs when strop reads it
//! (plan 0108 H9): fsmonitor, textconv, external diff and signature
//! checks are overridden on every git command strop starts.
use crate::exec::{with_token, GitExec};
use crate::log::{read_history, HistoryRead};
use crate::memory::{blame_file, blame_line, show_commit};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Hermetic git for building the fixture: no real HOME or global config.
fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join(".xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repository whose config names `evil` for every program-running
/// knob a read could reach; `evil` appends to `marker` when it runs.
fn hostile_repository(root: &Path, marker: &Path) -> String {
    let evil = root.join("evil.sh");
    std::fs::write(
        &evil,
        format!(
            "#!/bin/sh\necho \"$0 $*\" >> '{}'\ncat \"$1\" 2>/dev/null\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&evil, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@t.t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".gitattributes"), "*.txt diff=evil\n").unwrap();
    std::fs::write(repo.join("f.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "first"]);
    std::fs::write(repo.join("f.txt"), "one\ntwo\n").unwrap();
    git(&repo, &["commit", "-qam", "second"]);
    let program = evil.to_str().unwrap();
    for (key, value) in [
        ("core.fsmonitor", program),
        ("diff.evil.textconv", program),
        ("diff.evil.command", program),
        ("diff.external", program),
        ("log.showSignature", "true"),
        ("gpg.program", program),
        ("core.pager", program),
    ] {
        git(&repo, &["config", key, value]);
    }
    // a dirty worktree: status and blame look at it
    std::fs::write(repo.join("f.txt"), "one\ntwo\nthree\n").unwrap();
    git(&repo, &["rev-parse", "HEAD"])
}

#[test]
fn reads_never_run_the_repository_configured_programs() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("marker");
    let head = hostile_repository(directory.path(), &marker);
    let workdir = directory.path().join("repo");
    let exec = GitExec::Local { workdir: &workdir };
    let file = Path::new("f.txt");
    with_token(|token| {
        blame_line(&exec, &token, file, 1).unwrap();
        blame_file(&exec, &token, file).unwrap();
        show_commit(&exec, &token, &head).unwrap();
        for (file, range) in [(None, None), (Some(file), None), (Some(file), Some((1, 2)))] {
            let read = HistoryRead {
                exec: &exec,
                cancel: &token,
                max: 10,
                file,
                range,
                graph: None,
            };
            read_history(&read, |_| {}).unwrap();
        }
        let status: Vec<OsString> = vec!["status".into(), "--porcelain".into()];
        assert!(exec.run(&status, &token).unwrap().success);
    });
    assert!(
        !marker.exists(),
        "a repository program ran: {}",
        std::fs::read_to_string(&marker).unwrap_or_default()
    );
}
