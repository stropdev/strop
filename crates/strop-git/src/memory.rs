//! Git memory (M3, 0001 pillar 3.2/3.3): blame and a commit with its
//! changed files (the log is `log.rs`). Reads run through [`GitExec`] — local `git` or bounded remote
//! `git` over the shared execution boundary (0036 RW8) — with one argv
//! builder and one structured parser per query, so both backends admit
//! exactly the same machine formats. Permalinks and remote
//! normalization live in `permalink`/`ssh` (0033 finding 1).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use strop_core::worker::CancelToken;

use crate::exec::GitExec;

/// A blame card for one line (0001 pillar 3.3).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlameCard {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub age: String,
    pub summary: String,
    pub line: usize,
}

/// Blame one line of a file (1-based). Porcelain format.
pub fn blame_line(
    exec: &GitExec,
    cancel: &CancelToken,
    rel: &Path,
    line: usize,
) -> Result<BlameCard, String> {
    let argv: Vec<OsString> = vec![
        "blame".into(),
        "--line-porcelain".into(),
        "-L".into(),
        format!("{line},{line}").into(),
        "--".into(),
        rel.as_os_str().into(),
    ];
    let stdout = exec.run_records("git blame", &argv, cancel)?;
    let text = String::from_utf8_lossy(&stdout);
    let mut sha = String::new();
    let mut author = String::new();
    let mut summary = String::new();
    let mut ts = 0i64;
    for l in text.lines() {
        if sha.is_empty()
            && !l.starts_with('\t')
            && l.chars().take(8).all(|c| c.is_ascii_hexdigit())
        {
            sha = l.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(a) = l.strip_prefix("author ") {
            author = a.to_string();
        } else if let Some(t) = l.strip_prefix("author-time ") {
            ts = t.parse().unwrap_or(0);
        } else if let Some(s) = l.strip_prefix("summary ") {
            summary = s.to_string();
        }
    }
    if sha.is_empty() {
        return Err("no blame for line".into());
    }
    Ok(BlameCard {
        short_sha: sha.chars().take(8).collect(),
        sha,
        author,
        age: rel_age(ts),
        summary,
        line,
    })
}

/// One line of a whole-file blame (0001 pillar 3.3, the toggleable
/// column). `age` is rendered at parse time; `ts` keeps "recent"
/// honest for the caller's coloring.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlameLine {
    pub sha: String,
    pub author: String,
    /// Human short form ("3h", "2d", "5mo"); "now" when uncommitted.
    pub age: String,
    /// Author time, unix seconds (0 = uncommitted).
    pub ts: i64,
}

impl BlameLine {
    /// Worktree lines git blame attributes to nobody (all-zero sha).
    pub fn is_uncommitted(&self) -> bool {
        !self.sha.is_empty() && self.sha.chars().all(|c| c == '0')
    }
}

/// Blame every line of a file (`--line-porcelain`; the gutter's data,
/// 0011 §3). One bounded run on a job thread — never the input path.
pub fn blame_file(
    exec: &GitExec,
    cancel: &CancelToken,
    rel: &Path,
) -> Result<Vec<BlameLine>, String> {
    let argv: Vec<OsString> = vec![
        "blame".into(),
        "--line-porcelain".into(),
        "--".into(),
        rel.as_os_str().into(),
    ];
    let stdout = exec.run_records("git blame", &argv, cancel)?;
    let mut lines = Vec::new();
    let mut sha = String::new();
    let mut author = String::new();
    let mut ts = 0i64;
    for l in String::from_utf8_lossy(&stdout).lines() {
        if let Some(content) = l.strip_prefix('\t') {
            // the record's content row closes it — porcelain repeats
            // the full header per line, so every tab row emits one
            let _ = content;
            if !sha.is_empty() {
                let uncommitted = sha.chars().all(|c| c == '0');
                lines.push(BlameLine {
                    sha: sha.clone(),
                    age: if uncommitted {
                        "now".into()
                    } else {
                        rel_age(ts)
                    },
                    author: if uncommitted {
                        "you".into()
                    } else {
                        author.clone()
                    },
                    ts: if uncommitted { 0 } else { ts },
                });
            }
            sha.clear();
            author.clear();
            ts = 0;
        } else if sha.is_empty()
            && !l.is_empty()
            && l.chars().take(40).all(|c| c.is_ascii_hexdigit())
        {
            sha = l.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(a) = l.strip_prefix("author ") {
            author = a.to_string();
        } else if let Some(t) = l.strip_prefix("author-time ") {
            ts = t.parse().unwrap_or(0);
        }
    }
    if lines.is_empty() {
        return Err("no blame for file".into());
    }
    Ok(lines)
}

/// What happened to a file in a commit, as git's status letter says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileStatus {
    Added,
    Copied,
    Deleted,
    #[default]
    Modified,
    Renamed,
    TypeChanged,
    Unmerged,
    /// A letter git has and strop does not know (`X`, `B`).
    Other,
}

impl FileStatus {
    pub(crate) fn from_letter(letter: u8) -> Self {
        match letter {
            b'A' => Self::Added,
            b'C' => Self::Copied,
            b'D' => Self::Deleted,
            b'M' => Self::Modified,
            b'R' => Self::Renamed,
            b'T' => Self::TypeChanged,
            b'U' => Self::Unmerged,
            _ => Self::Other,
        }
    }

    /// The one-letter column git prints.
    pub fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Copied => 'C',
            Self::Deleted => 'D',
            Self::Modified => 'M',
            Self::Renamed => 'R',
            Self::TypeChanged => 'T',
            Self::Unmerged => 'U',
            Self::Other => 'X',
        }
    }
}

/// One file a commit changed: a row of the commit's file list (0100 §7).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChangedFile {
    /// The file's path after the commit — its identity.
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub added: usize,
    pub deleted: usize,
    pub status: FileStatus,
    /// Where a renamed or copied file came from.
    #[serde(default, with = "strop_core::path_serde::option")]
    pub old_path: Option<PathBuf>,
    /// Line counts do not apply: `added` and `deleted` are 0.
    pub binary: bool,
}

/// What a commit says about itself (0100 §7).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommitSummary {
    pub sha: String,
    pub author: String,
    /// Author date, unix seconds.
    pub time: i64,
    pub subject: String,
    /// The message after the subject and its blank line; empty when the
    /// message is one line.
    pub body: String,
}

/// A commit and the files it changed: the dive from the log.
#[derive(Debug, Clone, Default)]
pub struct CommitDetail {
    pub summary: CommitSummary,
    pub files: Vec<ChangedFile>,
}

/// A commit's message and changed files. Paths come from the
/// NUL-delimited machine forms, so native — never C-quoted, possibly
/// non-UTF8 — names arrive as the worktree identities `commit_file_diff`
/// expects. Two bounded runs: the message, then the raw and numstat
/// records.
pub fn show_commit(
    exec: &GitExec,
    cancel: &CancelToken,
    sha: &str,
) -> Result<CommitDetail, String> {
    let argv: Vec<OsString> = vec![
        "show".into(),
        "-s".into(),
        "--format=%H%x00%an%x00%at%x00%s%x00%b".into(),
        sha.into(),
    ];
    let header = exec.run_records("git show", &argv, cancel)?;
    let summary = parse_summary(&header)?;
    let argv: Vec<OsString> = vec![
        "show".into(),
        "--raw".into(),
        "--numstat".into(),
        "-z".into(),
        "--format=".into(),
        sha.into(),
    ];
    let stdout = exec.run_records("git show", &argv, cancel)?;
    Ok(CommitDetail {
        summary,
        files: crate::numstat::parse_show(&stdout)?,
    })
}

/// `%H %an %at %s %b`, NUL-separated; the body is last, so nothing in
/// it can shift a field.
fn parse_summary(bytes: &[u8]) -> Result<CommitSummary, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.splitn(5, '\0');
    let mut next = || fields.next().ok_or("git show: malformed commit header");
    let sha = next()?.to_string();
    let author = next()?.to_string();
    let time = next()?
        .parse()
        .map_err(|_| "git show: malformed author date".to_string())?;
    let subject = next()?.to_string();
    let body = next()?.trim_end().to_string();
    Ok(CommitSummary {
        sha,
        author,
        time,
        subject,
        body,
    })
}

/// Relative age, human short form ("3h", "2d", "5mo").
fn rel_age(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = (now - ts).max(0);
    match age {
        a if a < 3600 => format!("{}m", a / 60),
        a if a < 86400 => format!("{}h", a / 3600),
        a if a < 86400 * 30 => format!("{}d", a / 86400),
        a if a < 86400 * 365 => format!("{}mo", a / (86400 * 30)),
        a => format!("{}y", a / (86400 * 365)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::with_token;
    use crate::Repo;

    /// The local backend for a repo root — what every local memory
    /// call site constructs.
    fn local(root: &Path) -> GitExec<'_> {
        GitExec::Local { workdir: root }
    }

    /// Repo with two commits (f.rs grows a line), then a dirty edit —
    /// blame_file must attribute committed lines and flag dirty ones.
    #[test]
    fn blame_file_attributes_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.rs"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "first"]);
        std::fs::write(root.join("f.rs"), "one\ntwo\n").unwrap();
        git(&["commit", "-qam", "second"]);

        let clean =
            with_token(|token| blame_file(&local(root), &token, Path::new("f.rs"))).unwrap();
        assert_eq!(clean.len(), 2, "one BlameLine per file line");
        assert_eq!(clean[0].author, "t");
        assert_eq!(clean[1].author, "t");
        assert_ne!(clean[0].sha, clean[1].sha, "two commits, two shas");
        assert!(!clean[0].is_uncommitted());

        // dirty worktree: the new line belongs to nobody
        std::fs::write(root.join("f.rs"), "one\ntwo\nthree\n").unwrap();
        let dirty =
            with_token(|token| blame_file(&local(root), &token, Path::new("f.rs"))).unwrap();
        assert_eq!(dirty.len(), 3);
        assert!(dirty[2].is_uncommitted(), "last line is uncommitted");
        assert_eq!(dirty[2].age, "now");
        assert_eq!(dirty[2].author, "you");
        assert_eq!(dirty[2].ts, 0);
    }

    #[test]
    fn blame_file_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            with_token(|token| blame_file(&local(dir.path()), &token, Path::new("nope.rs")))
                .is_err()
        );
    }

    /// Hermetic git: no reads of the real HOME or system/global config,
    /// no network, no sleeps. Returns trimmed stdout for rev-parse.
    fn git_here(root: &Path, args: &[&str]) -> String {
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
            String::from_utf8_lossy(&out.stderr).trim()
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository with the identity set and one commit per call.
    fn commit_all(root: &Path, message: &str) -> String {
        git_here(root, &["add", "-A"]);
        git_here(root, &["commit", "-qm", message]);
        git_here(root, &["rev-parse", "HEAD"])
    }

    fn init(root: &Path) {
        git_here(root, &["init", "-q", "-b", "main"]);
        git_here(root, &["config", "user.email", "t@t.t"]);
        git_here(root, &["config", "user.name", "t"]);
    }

    fn files_of(root: &Path, sha: &str) -> Vec<ChangedFile> {
        with_token(|token| show_commit(&local(root), &token, sha))
            .unwrap()
            .files
    }

    /// The review-repo bug: `src/日本語.rs` reached ChangedFiles as a
    /// C-quoted octal escape and renames as `old => new`, neither a
    /// worktree identity. Under `-z` the native names must come back —
    /// ordinary, Unicode, rename (destination, with where it came from)
    /// and binary rows all present, each with git's status letter.
    #[test]
    fn commit_files_keep_native_paths_and_say_what_happened() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("a.rs"), "one\n").unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\n").unwrap();
        std::fs::write(root.join("ren.txt"), "old\nlines\nkept\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"\0\x01binary\0").unwrap();
        std::fs::write(root.join("gone.txt"), "bye\n").unwrap();
        commit_all(root, "first");
        git_here(root, &["mv", "ren.txt", "new.txt"]);
        std::fs::write(root.join("a.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\nfn y() {}\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"\0\x01changed\0").unwrap();
        std::fs::write(root.join("fresh.txt"), "hi\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        let sha = commit_all(root, "second");

        let files = files_of(root, &sha);
        assert_eq!(files.len(), 6, "{files:?}");
        let row = |p: &str| {
            files
                .iter()
                .find(|f| f.path == Path::new(p))
                .unwrap_or_else(|| panic!("missing {p} in {files:?}"))
        };
        assert_eq!((row("a.rs").added, row("a.rs").deleted), (2, 0));
        assert_eq!(row("a.rs").status, FileStatus::Modified);
        // the Unicode identity arrives native, never `"src/\346..."`
        assert_eq!(row("src/日本語.rs").added, 1);
        // rename: the destination is the row, and it knows its source
        assert_eq!(row("new.txt").status, FileStatus::Renamed);
        assert_eq!(
            row("new.txt").old_path.as_deref(),
            Some(Path::new("ren.txt"))
        );
        assert!(!files.iter().any(|f| f.path == Path::new("ren.txt")));
        // binary: the row survives, flagged, with 0/0 counts
        assert!(row("bin.dat").binary);
        assert_eq!((row("bin.dat").added, row("bin.dat").deleted), (0, 0));
        assert!(!row("a.rs").binary);
        assert_eq!(row("fresh.txt").status, FileStatus::Added);
        assert_eq!(row("gone.txt").status, FileStatus::Deleted);
        assert_eq!(row("gone.txt").status.letter(), 'D');
        assert!(files
            .iter()
            .all(|f| !f.path.to_string_lossy().starts_with('"')));
    }

    /// The commit's own words come with its files: subject, body,
    /// author and author date.
    #[test]
    fn a_commit_says_who_when_and_why() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::write(root.join("f.txt"), "one\n").unwrap();
        git_here(root, &["add", "-A"]);
        git_here(
            root,
            &[
                "commit",
                "-qm",
                "the subject",
                "-m",
                "a body\n\nin two paragraphs",
            ],
        );
        let sha = git_here(root, &["rev-parse", "HEAD"]);
        let time: i64 = git_here(root, &["log", "-1", "--format=%at"])
            .parse()
            .unwrap();
        let detail = with_token(|token| show_commit(&local(root), &token, &sha)).unwrap();
        assert_eq!(
            detail.summary,
            CommitSummary {
                sha: sha.clone(),
                author: "t".into(),
                time,
                subject: "the subject".into(),
                body: "a body\n\nin two paragraphs".into(),
            }
        );
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].status, FileStatus::Added);
    }

    /// A merge's files are the ones today's list showed (numstat lists
    /// the rows); its combined raw records only add the letters.
    #[test]
    fn a_merge_lists_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::write(root.join("m.txt"), "a\n").unwrap();
        commit_all(root, "one");
        git_here(root, &["checkout", "-qb", "side"]);
        std::fs::write(root.join("z.txt"), "z\n").unwrap();
        commit_all(root, "side");
        git_here(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("m.txt"), "a\nb\n").unwrap();
        commit_all(root, "main moves");
        git_here(root, &["merge", "-q", "--no-edit", "side"]);
        let sha = git_here(root, &["rev-parse", "HEAD"]);
        let argv: Vec<OsString> = vec![
            "show".into(),
            "--numstat".into(),
            "-z".into(),
            "--format=".into(),
            sha.clone().into(),
        ];
        let numstat =
            with_token(|token| local(root).run_records("git show", &argv, &token)).unwrap();
        let today = crate::numstat::parse_numstat(&numstat).unwrap();
        let files = files_of(root, &sha);
        let paths = |rows: &[ChangedFile]| rows.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&files), paths(&today));
    }

    /// The file list's paths are real identities: hand one straight to
    /// the git2-based diff the dive opens next.
    #[test]
    fn commit_file_paths_feed_commit_file_diff() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\n").unwrap();
        commit_all(root, "first");
        std::fs::write(
            root.join("src/日本語.rs"),
            "fn x() {}\nfn y() {}\nfn z() {}\n",
        )
        .unwrap();
        let sha = commit_all(root, "second");

        let files = files_of(root, &sha);
        let uni = files
            .iter()
            .find(|f| f.path == Path::new("src/日本語.rs"))
            .expect("native unicode path is a row");
        let repo = Repo::discover(root).unwrap();
        let diff = repo.commit_file_diff(&sha, &uni.path).unwrap();
        assert_eq!(diff.added, 2);
        assert_eq!(diff.deleted, 0);
    }

    /// Unix filenames may be non-UTF8; they must arrive byte-for-byte,
    /// not through a quoted or lossy spelling.
    #[cfg(unix)]
    #[test]
    fn commit_files_preserve_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::create_dir(root.join("src")).unwrap();
        let name = std::ffi::OsStr::from_bytes(b"src/\xff\xfe.rs");
        std::fs::write(root.join(name), "fn x() {}\n").unwrap();
        let sha = commit_all(root, "first");

        let files = files_of(root, &sha);
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path.as_os_str().as_bytes(), b"src/\xff\xfe.rs");
        assert_eq!(files[0].added, 1);
    }
}
