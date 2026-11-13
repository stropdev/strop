use super::*;
use crate::exec::with_token;
use std::process::Command;

/// Hermetic git: no reads of the real HOME or system/global config.
pub(crate) fn git_here(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join(".xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ånne Author")
        .env("GIT_AUTHOR_EMAIL", "a@example.com")
        .env("GIT_COMMITTER_NAME", "C")
        .env("GIT_COMMITTER_EMAIL", "c@example.com")
        // a fixed date: the rows hold still between two reads
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A history with a merge and a branch, a tag, and a subject that holds
/// the row's own separator ` · ` and non-ASCII text.
pub(crate) fn repository(root: &Path) {
    git_here(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("f.txt"), "one\n").unwrap();
    git_here(root, &["add", "-A"]);
    git_here(root, &["commit", "-qm", "first · with a dot"]);
    git_here(root, &["tag", "v1"]);
    git_here(root, &["checkout", "-qb", "side"]);
    std::fs::write(root.join("s.txt"), "side\n").unwrap();
    git_here(root, &["add", "-A"]);
    git_here(root, &["commit", "-qm", "side: naïve"]);
    git_here(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("f.txt"), "one\ntwo\n").unwrap();
    git_here(root, &["commit", "-qam", "main moves"]);
    git_here(root, &["merge", "-q", "--no-edit", "side"]);
}

fn read(root: &Path, file: Option<&Path>, range: Option<(usize, usize)>) -> Vec<LogRow> {
    let exec = GitExec::Local { workdir: root };
    with_token(|token| {
        history(&HistoryRead {
            exec: &exec,
            cancel: &token,
            max: 200,
            file,
            range,
            graph: None,
        })
    })
    .unwrap()
}

/// Each row as git prints it with `--graph`: the graph runes, then the
/// commit's fields, whatever the merges and connector rows.
fn spelled(rows: &[LogRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match &row.commit {
            Some(commit) => format!("{}{} {}", row.graph, commit.short, commit.subject),
            None => row.graph.clone(),
        })
        .collect()
}

#[test]
fn the_typed_rows_are_the_rows_git_printed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root);
    let rows = read(root, None, None);
    let printed = git_here(root, &["log", "--format=%h %s", "-n", "200", "--graph"]);
    assert_eq!(spelled(&rows), printed.lines().collect::<Vec<_>>());
    let file = read(root, Some(Path::new("f.txt")), None);
    let printed = git_here(
        root,
        &[
            "log",
            "--format=%h %s",
            "-n",
            "200",
            "--graph",
            "--",
            "f.txt",
        ],
    );
    assert_eq!(spelled(&file), printed.lines().collect::<Vec<_>>());
}

#[test]
fn rows_carry_typed_fields() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root);
    let rows = read(root, None, None);
    let head = rows[0].commit.as_ref().expect("the first row is a commit");
    assert_eq!(head.sha, git_here(root, &["rev-parse", "HEAD"]).trim());
    assert_eq!(
        head.short,
        git_here(root, &["rev-parse", "--short", "HEAD"]).trim()
    );
    assert_eq!(head.refs, ["HEAD -> main"]);
    assert_eq!(head.author, "Ånne Author");
    let time: i64 = git_here(root, &["log", "-1", "--format=%at"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(head.time, time);
    assert!(head.subject.starts_with("Merge branch 'side'"));
    assert_eq!(rows[0].graph, "*   ");
    // a connector row is graph only
    let connector = rows.iter().find(|row| row.commit.is_none()).unwrap();
    assert!(connector.graph.contains(['|', '/', '\\']));
    assert!(connector.sha().is_none());
    let first = rows
        .iter()
        .rev()
        .find_map(|row| row.commit.as_ref())
        .unwrap();
    assert_eq!(first.subject, "first · with a dot");
    assert_eq!(first.refs, ["tag: v1"]);
    let side = rows
        .iter()
        .filter_map(|row| row.commit.as_ref())
        .find(|commit| commit.subject == "side: naïve")
        .unwrap();
    assert_eq!(side.refs, ["side"]);
}

#[test]
fn a_line_range_lists_only_its_commits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root);
    let rows = read(root, Some(Path::new("f.txt")), Some((2, 2)));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].graph, "");
    assert_eq!(rows[0].commit.as_ref().unwrap().subject, "main moves");
    // a range without a file is refused; a bad one is git's error
    let exec = GitExec::Local { workdir: root };
    let ask = |file: Option<&Path>, range| {
        with_token(|token| {
            history(&HistoryRead {
                exec: &exec,
                cancel: &token,
                max: 10,
                file,
                range,
                graph: None,
            })
        })
    };
    assert!(ask(None, Some((1, 1))).is_err());
    assert!(ask(Some(Path::new("absent.txt")), Some((1, 1))).is_err());
}

/// Whatever the chunk boundaries — mid-row, mid-UTF-8, on a separator,
/// one byte at a time — the rows are the rows of the whole stream.
#[test]
fn rows_do_not_depend_on_chunk_boundaries() {
    let stream = "*   \x1eaaaa\x1fa\x1fÅnne\x1f100\x1fHEAD -> main, tag: v1\x1fmerge ·\n\
                  |\\  \n\
                  | * \x1ebbbb\x1fb\x1fZoë\x1f90\x1f\x1fnaïve\n\
                  * | \x1ecccc\x1fc\x1fX\x1f80\x1f\x1flast without newline";
    let bytes = stream.as_bytes();
    let whole = {
        let mut records = LogRecords::new(false);
        let mut rows = records.push(bytes);
        rows.extend(records.finish());
        rows
    };
    assert_eq!(whole.len(), 4);
    assert_eq!(
        whole[3].commit.as_ref().unwrap().subject,
        "last without newline"
    );
    for size in 1..=bytes.len() {
        let mut records = LogRecords::new(false);
        let mut rows = Vec::new();
        for chunk in bytes.chunks(size) {
            rows.extend(records.push(chunk));
        }
        rows.extend(records.finish());
        assert_eq!(rows, whole, "chunk size {size}");
    }
    // every split point, two chunks
    for cut in 0..=bytes.len() {
        let mut records = LogRecords::new(false);
        let mut rows = records.push(&bytes[..cut]);
        rows.extend(records.push(&bytes[cut..]));
        rows.extend(records.finish());
        assert_eq!(rows, whole, "cut at {cut}");
    }
}

/// The streamed run hands over the same rows the whole run collects.
#[test]
fn streamed_pages_concatenate_to_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root);
    let exec = GitExec::Local { workdir: root };
    let mut pages = Vec::new();
    with_token(|token| {
        read_history(
            &HistoryRead {
                exec: &exec,
                cancel: &token,
                max: 200,
                file: None,
                range: None,
                graph: None,
            },
            |page| pages.push(page),
        )
    })
    .unwrap();
    assert_eq!(pages.concat(), read(root, None, None));
}
