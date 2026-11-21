//! Blame navigation and `Space g l` on the real binary. On a large
//! repository (plans/0101 §5.1) the first press leaves a private commit-graph in
//! strop's state directory — not in the repository — and the second
//! press shows exactly the same log.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Enough commits that git's `log --graph` walk takes over the 100 ms
/// that asks for a graph.
const COMMITS: usize = 80_000;

fn git(root: &Path, args: &[&str], stdin: Option<&[u8]>) {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes).unwrap();
    }
    assert!(child.wait().unwrap().success(), "git {args:?}");
}

/// A linear history, one of 200 files changed per commit, old dates (the
/// log's ages hold still between the two presses).
fn repository(root: &Path) {
    let mut stream = Vec::new();
    for n in 0..COMMITS {
        let when = 1_500_000_000 + n * 600;
        let path = format!("src/f{:03}.txt", n % 200);
        let msg = format!("commit {n}\n");
        let body = format!("line {n}\n");
        stream.extend_from_slice(
            format!(
                "commit refs/heads/main\n\
                 author A <a@example.com> {when} +0000\n\
                 committer A <a@example.com> {when} +0000\n\
                 data {}\n{msg}",
                msg.len()
            )
            .as_bytes(),
        );
        stream.extend_from_slice(
            format!("M 100644 inline {path}\ndata {}\n{body}\n", body.len()).as_bytes(),
        );
    }
    git(root, &["init", "-q", "-b", "main"], None);
    git(root, &["fast-import", "--quiet"], Some(&stream));
    git(root, &["reset", "-q", "--hard", "main"], None);
}

fn run(root: &Path, state: &Path, options: &[&str], file: &str, script: &str) -> String {
    let outside = root.parent().unwrap();
    let steps = outside.join("steps");
    std::fs::write(&steps, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", outside.join("home"))
        .env("XDG_CONFIG_HOME", outside.join("config"))
        .env("XDG_STATE_HOME", state)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("STROP_LOG")
        .args(options)
        .arg("headless")
        .arg(&steps)
        .arg(file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn frames(output: &str) -> Vec<&str> {
    output.split("─── frame ").skip(1).collect()
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                found.extend(files(&entry.path()));
            } else {
                found.push(entry.path());
            }
        }
    }
    found
}

#[test]
fn a_slow_log_leaves_a_private_graph_and_the_log_reads_the_same() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    repository(root);
    let state = directory.path().join("state");
    let before = files(&root.join(".git"));
    let output = run(
        root,
        &state,
        &[],
        "src/f007.txt",
        "settle 60000\n\
         keys <space>gl\n\
         settle 60000\n\
         frame\n\
         keys q\n\
         settle 60000\n\
         keys <space>gl\n\
         settle 60000\n\
         frame\n",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 2, "{output}");
    assert!(frames[0].contains("commit 79999"), "{}", frames[0]);
    assert_eq!(frames[0], frames[1], "the graph changes no row");
    let graphs: Vec<_> = files(&state.join("strop/graphs"))
        .into_iter()
        .filter(|path| path.ends_with("objects/info/commit-graph"))
        .collect();
    assert_eq!(graphs.len(), 1, "one graph in the state directory");
    assert_eq!(files(&root.join(".git")), before, "nothing new in .git");
    assert!(!root.join(".git/objects/info/commit-graph").exists());
}

#[test]
fn blame_card_enter_requires_an_attributed_commit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"], None);
    // The anchor belongs to the first commit, not HEAD: opening a log
    // at its default selection cannot satisfy the committed-line control.
    git(
        &root,
        &["fast-import", "--quiet"],
        Some(
            b"commit refs/heads/main\n\
              committer Ada <ada@example.org> 1500000000 +0000\n\
              data <<END\nanchor origin\nEND\n\
              M 100644 inline note\ndata <<END\ncommitted\nanchor\nEND\n\
              commit refs/heads/main\n\
              committer Ada <ada@example.org> 1500000001 +0000\n\
              data <<END\nunrelated tip\nEND\n\
              M 100644 inline other\ndata <<END\nother\nEND\n",
        ),
    );
    git(&root, &["reset", "-q", "--hard", "main"], None);
    std::fs::write(root.join("note"), "edited on disk\nanchor\n").unwrap();
    let output = run(
        &root,
        &directory.path().join("state"),
        &["-R"],
        "note",
        "settle\nkeys <space>gB\nsettle\nframe\n\
         keys <cr>\nsettle\nframe\n\
         keys j<space>gB\nsettle\nframe\n\
         keys <cr>\nsettle\nframe\n\
         keys <cr>\nsettle\nframe\n",
    );
    let frames = frames(&output);
    let [dirty_card, source, clean_card, log, commit] = frames.as_slice() else {
        panic!("{output}");
    };
    assert!(dirty_card.contains("Not Committed Yet"), "{dirty_card}");
    assert!(dirty_card.contains("dismiss"), "{dirty_card}");
    assert!(!dirty_card.contains("dive"), "{dirty_card}");
    assert!(source.contains("edited on disk"), "{source}");
    assert!(source.contains("uncommitted line"), "{source}");
    assert!(!source.contains("git log"), "{source}");
    assert!(clean_card.contains("anchor origin"), "{clean_card}");
    assert!(clean_card.contains("dive"), "{clean_card}");
    assert!(log.contains("git log"), "{log}");
    assert!(commit.contains("anchor origin"), "{commit}");
    assert!(!commit.contains("unrelated tip"), "{commit}");
}
