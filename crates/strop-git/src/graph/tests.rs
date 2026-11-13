use super::*;
use crate::exec::{with_token, GitExec};
use crate::log::{history, HistoryRead, LogRow};
use std::io::Write;
use std::process::{Command, Stdio};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `count` commits on `main` after `from` (or a root), one file of
/// twenty touched each, a one-commit side branch merged every tenth.
fn append(root: &Path, start: usize, count: usize) {
    let mut stream = Vec::new();
    for n in start..start + count {
        let when = 1_500_000_000 + n * 600;
        let path = format!("src/f{:02}.txt", n % 20);
        let body = format!("line {n}\n");
        if n % 10 == 9 {
            let msg = format!("side {n}\n");
            stream.extend_from_slice(b"commit refs/heads/side\n");
            stream.extend(format!("committer t <t@example.com> {when} +0000\n").bytes());
            stream.extend(format!("data {}\n{msg}", msg.len()).bytes());
            stream.extend_from_slice(b"from refs/heads/main\n");
            stream
                .extend(format!("M 100644 inline side.txt\ndata {}\n{body}\n", body.len()).bytes());
        }
        let msg = format!("commit {n}\n");
        stream.extend_from_slice(b"commit refs/heads/main\n");
        stream.extend(format!("committer t <t@example.com> {when} +0000\n").bytes());
        stream.extend(format!("data {}\n{msg}", msg.len()).bytes());
        if n == start && start > 0 {
            // continue the repository's branch, not a new root
            stream.extend_from_slice(b"from refs/heads/main^0\n");
        }
        if n % 10 == 9 {
            stream.extend_from_slice(b"merge refs/heads/side\n");
        }
        stream.extend(format!("M 100644 inline {path}\ndata {}\n{body}\n", body.len()).bytes());
    }
    let mut child = Command::new("git")
        .args(["fast-import", "--quiet", "--force"])
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&stream).unwrap();
    assert!(child.wait().unwrap().success());
    git(root, &["reset", "-q", "--hard", "main"]);
}

fn repository(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    append(dir.path(), 0, count);
    dir
}

const FILE: &str = "src/f07.txt";

fn rows(root: &Path, graph: Option<&Path>, file: Option<&Path>) -> Vec<LogRow> {
    let exec = GitExec::Local { workdir: root };
    with_token(|token| {
        history(&HistoryRead {
            exec: &exec,
            cancel: &token,
            max: 1000,
            file,
            range: None,
            graph,
        })
    })
    .unwrap()
}

fn build_both(root: &Path, side: &Path) {
    for step in [Step::Reachable, Step::ChangedPaths] {
        with_token(|token| build(step, root, side, &token)).unwrap();
    }
}

/// Names, sizes and modification times of everything under `.git`.
fn snapshot(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    fn walk(dir: &Path, into: &mut Vec<(PathBuf, u64, SystemTime)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                walk(&entry.path(), into);
            }
            into.push((entry.path(), metadata.len(), metadata.modified().unwrap()));
        }
    }
    let mut files = Vec::new();
    walk(&root.join(".git"), &mut files);
    files.sort();
    files
}

/// The load-bearing one (0101 §5.1): a private graph — fresh, behind,
/// or naming commits the repository has since pruned — never changes a
/// row of the log or of a file's history.
#[test]
fn a_private_graph_never_changes_a_row() {
    let dir = repository(300);
    let root = dir.path();
    let state = tempfile::tempdir().unwrap();
    let side = side_dir(&graphs_root(state.path()), &common_objects(root).unwrap());
    let objects = side.join("objects");
    let check = |what: &str| {
        for file in [None, Some(Path::new(FILE))] {
            let plain = rows(root, None, file);
            assert!(plain.len() > 10, "{what}: a real history");
            assert_eq!(
                rows(root, Some(&objects), file),
                plain,
                "{what}, file {file:?}"
            );
        }
    };

    build_both(root, &side);
    assert!(objects.join("info/commit-graph").is_file());
    check("fresh");

    // 200 commits behind
    append(root, 300, 200);
    check("behind");

    // names commits the repository has reset away and pruned
    let state2 = tempfile::tempdir().unwrap();
    let side2 = side_dir(&graphs_root(state2.path()), &common_objects(root).unwrap());
    build_both(root, &side2);
    git(root, &["reset", "-q", "--hard", "HEAD~40"]);
    git(root, &["reflog", "expire", "--expire=now", "--all"]);
    git(
        root,
        &["-c", "gc.writeCommitGraph=false", "gc", "-q", "--prune=now"],
    );
    append(root, 1000, 2);
    assert!(!owns_graph(&common_objects(root).unwrap()));
    for file in [None, Some(Path::new(FILE))] {
        let plain = rows(root, None, file);
        assert_eq!(
            rows(root, Some(&side2.join("objects")), file),
            plain,
            "pruned, file {file:?}"
        );
    }
}

/// strop writes nothing into the user's repository — no file, no lock —
/// and installs by rename, leaving no scratch behind.
#[test]
fn a_build_leaves_the_repository_untouched() {
    let dir = repository(60);
    let root = dir.path();
    let before = snapshot(root);
    let state = tempfile::tempdir().unwrap();
    let side = side_dir(&graphs_root(state.path()), &common_objects(root).unwrap());
    build_both(root, &side);
    assert_eq!(snapshot(root), before);
    assert!(!owns_graph(&common_objects(root).unwrap()));
    let left: Vec<_> = std::fs::read_dir(&side)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(left, ["objects"], "no scratch directory remains");
    // a second build reuses the installed file and installs again
    with_token(|token| build(Step::ChangedPaths, root, &side, &token)).unwrap();
    assert!(side.join("objects/info/commit-graph").is_file());
    assert_eq!(snapshot(root), before);
}

#[test]
fn a_repository_with_its_own_graph_is_gits() {
    let dir = repository(30);
    let root = dir.path();
    let objects = common_objects(root).unwrap();
    assert!(!owns_graph(&objects));
    git(root, &["commit-graph", "write", "--reachable"]);
    assert!(owns_graph(&objects));
}

#[test]
fn history_reads_name_the_graph_only_once_it_exists() {
    let dir = repository(30);
    let root = dir.path();
    let state = tempfile::tempdir().unwrap();
    let graphs = graphs_root(state.path());
    assert_eq!(for_history(&graphs, root), None);
    let side = side_dir(&graphs, &common_objects(root).unwrap());
    build_both(root, &side);
    assert_eq!(for_history(&graphs, root), Some(side.join("objects")));
}

/// The seventeenth repository makes room by removing the directory used
/// least recently.
#[test]
fn the_least_recently_used_graph_is_evicted() {
    let state = tempfile::tempdir().unwrap();
    let graphs = graphs_root(state.path());
    let base = SystemTime::now() - Duration::from_secs(10_000);
    for n in 0..KEEP {
        let dir = graphs.join(format!("repo{n:02}"));
        std::fs::create_dir_all(&dir).unwrap();
        // repo05 is the oldest
        let used = base + Duration::from_secs(if n == 5 { 0 } else { 100 + n as u64 });
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(used)
            .unwrap();
    }
    let new = graphs.join("new");
    evict(&graphs, &new, KEEP);
    let mut left: Vec<String> = std::fs::read_dir(&graphs)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left.len(), KEEP - 1);
    assert!(!left.contains(&"repo05".to_string()), "{left:?}");
    // a repository already kept evicts nothing
    let kept = graphs.join("repo01");
    evict(&graphs, &kept, KEEP);
    assert_eq!(std::fs::read_dir(&graphs).unwrap().count(), KEEP - 1);
}
