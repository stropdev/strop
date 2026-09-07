//! Git memory (M3, 0001 pillar 3.2/3.3): log graph, blame, changed-file
//! stats. Reads via shell `git` (matches user config; not hot-path).
//! Permalinks and remote normalization live in `permalink`/`ssh`
//! (0033 finding 1).

use std::path::{Path, PathBuf};

/// One log line from `git log --graph`, with the commit hash extracted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogRow {
    /// The rendered graph+summary line (what the buffer shows).
    pub text: String,
    /// Full SHA when the line names a commit (graph-only lines: None).
    pub sha: Option<String>,
}

/// `git log --graph` for the browser. Shells out — the log is not a
/// per-keystroke path (0001 §3). Caller decides threading.
pub fn log_graph(workdir: &Path, max: usize, file: Option<&Path>) -> Result<Vec<LogRow>, String> {
    log_graph_range(workdir, max, file, None)
}

/// `git log -L start,end:path` — the history of a line range (0014 wave
/// 4: selection archaeology). The graph flag is meaningless with -L;
/// rows come straight from the patch headers.
pub fn log_graph_range(
    workdir: &Path,
    max: usize,
    file: Option<&Path>,
    range: Option<(usize, usize)>,
) -> Result<Vec<LogRow>, String> {
    let mut cmd = std::process::Command::new("git");
    let (marker_fmt, ranged) = match range {
        Some(_) => ("%x01%h %an · %ar · %s%x00%H", true),
        None => ("%h %an · %ar · %s%x00%H", false),
    };
    // workdir and file operands pass as OsStr: a non-UTF8 repo path or
    // tracked filename must reach git byte-for-byte, not via a lossy
    // display() rendering
    cmd.arg("-C").arg(workdir).args([
        "log",
        &format!("--format={marker_fmt}"),
        "-n",
        &max.to_string(),
    ]);
    match (file, range) {
        (Some(f), Some((a, b))) => {
            // -L embeds the path in one argument; compose the OsString
            // instead of formatting through display()
            let mut spec = std::ffi::OsString::from(format!("-L{a},{b}:"));
            spec.push(f);
            cmd.arg(spec);
        }
        (Some(f), None) => {
            cmd.arg("--graph").arg("--").arg(f);
        }
        (None, None) => {
            cmd.arg("--graph");
        }
        (None, Some(_)) => return Err("-L needs a file".into()),
    }
    let out = cmd.output().map_err(|e| format!("spawn git log: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        // -L output carries patch text; only marked lines are commits
        .filter(|line| !ranged || line.starts_with('\x01'))
        .map(|line| {
            let line = line.strip_prefix('\x01').unwrap_or(line);
            // the format hides the full SHA after a NUL
            let (vis, sha) = match line.split_once('\0') {
                Some((v, s)) => (v.to_string(), Some(s.trim().to_string())),
                None => (line.to_string(), None),
            };
            LogRow { text: vis, sha }
        })
        .collect())
}

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

/// Blame one line of a file (1-based). Shells out; porcelain format.
pub fn blame_line(workdir: &Path, rel: &Path, line: usize) -> Result<BlameCard, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(["blame", "--line-porcelain", "-L", &format!("{line},{line}")])
        .arg("--")
        .arg(rel)
        .output()
        .map_err(|e| format!("spawn git blame: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
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
/// 0011 §3). Shells out on a job thread — never the input path.
pub fn blame_file(workdir: &Path, rel: &Path) -> Result<Vec<BlameLine>, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(["blame", "--line-porcelain"])
        .arg("--")
        .arg(rel)
        .output()
        .map_err(|e| format!("spawn git blame: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let mut lines = Vec::new();
    let mut sha = String::new();
    let mut author = String::new();
    let mut ts = 0i64;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
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

/// Files changed by a commit: `path | +N -M` rows for the dive view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChangedFile {
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub added: usize,
    pub deleted: usize,
}

/// Files changed by a commit: `path | +N -M` rows for the dive view.
/// Paths come from numstat's NUL-delimited machine form, so native —
/// never C-quoted, possibly non-UTF8 — names arrive as the worktree
/// identities `commit_file_diff` expects.
pub fn show_stat(workdir: &Path, sha: &str) -> Result<Vec<ChangedFile>, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(["show", "--numstat", "-z", "--format=", sha])
        .output()
        .map_err(|e| format!("spawn git show: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    crate::numstat::parse_numstat(&out.stdout)
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
    use crate::Repo;

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

        let clean = blame_file(root, Path::new("f.rs")).unwrap();
        assert_eq!(clean.len(), 2, "one BlameLine per file line");
        assert_eq!(clean[0].author, "t");
        assert_eq!(clean[1].author, "t");
        assert_ne!(clean[0].sha, clean[1].sha, "two commits, two shas");
        assert!(!clean[0].is_uncommitted());

        // dirty worktree: the new line belongs to nobody
        std::fs::write(root.join("f.rs"), "one\ntwo\nthree\n").unwrap();
        let dirty = blame_file(root, Path::new("f.rs")).unwrap();
        assert_eq!(dirty.len(), 3);
        assert!(dirty[2].is_uncommitted(), "last line is uncommitted");
        assert_eq!(dirty[2].age, "now");
        assert_eq!(dirty[2].author, "you");
        assert_eq!(dirty[2].ts, 0);
    }

    #[test]
    fn blame_file_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(blame_file(dir.path(), Path::new("nope.rs")).is_err());
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

    /// The review-repo bug: `src/日本語.rs` reached ChangedFiles as a
    /// C-quoted octal escape and renames as `old => new`, neither a
    /// worktree identity. Under `-z` the native names must come back —
    /// ordinary, Unicode, rename (destination only) and binary rows all
    /// present.
    #[test]
    fn show_stat_keeps_native_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_here(root, &["init", "-q"]);
        git_here(root, &["config", "user.email", "t@t.t"]);
        git_here(root, &["config", "user.name", "t"]);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("a.rs"), "one\n").unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\n").unwrap();
        std::fs::write(root.join("ren.txt"), "old\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"\0\x01binary\0").unwrap();
        git_here(root, &["add", "."]);
        git_here(root, &["commit", "-qm", "first"]);
        git_here(root, &["mv", "ren.txt", "new.txt"]);
        std::fs::write(root.join("a.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\nfn y() {}\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"\0\x01changed\0").unwrap();
        git_here(root, &["add", "."]);
        git_here(root, &["commit", "-qm", "second"]);
        let sha = git_here(root, &["rev-parse", "HEAD"]);

        let files = show_stat(root, &sha).unwrap();
        assert_eq!(files.len(), 4, "{files:?}");
        let row = |p: &str| {
            files
                .iter()
                .find(|f| f.path == Path::new(p))
                .unwrap_or_else(|| panic!("missing {p} in {files:?}"))
        };
        assert_eq!(row("a.rs").added, 2);
        assert_eq!(row("a.rs").deleted, 0);
        // the Unicode identity arrives native, never `"src/\346..."`
        assert_eq!(row("src/日本語.rs").added, 1);
        // rename: only the destination is a row
        assert_eq!(row("new.txt").added, 0);
        assert!(!files.iter().any(|f| f.path == Path::new("ren.txt")));
        // binary: the row survives with deliberate 0/0 counts
        assert_eq!((row("bin.dat").added, row("bin.dat").deleted), (0, 0));
        assert!(files
            .iter()
            .all(|f| !f.path.to_string_lossy().starts_with('"')));
    }

    /// show_stat's paths are real identities: hand one straight to the
    /// git2-based diff the dive opens next.
    #[test]
    fn show_stat_paths_feed_commit_file_diff() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_here(root, &["init", "-q"]);
        git_here(root, &["config", "user.email", "t@t.t"]);
        git_here(root, &["config", "user.name", "t"]);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/日本語.rs"), "fn x() {}\n").unwrap();
        git_here(root, &["add", "."]);
        git_here(root, &["commit", "-qm", "first"]);
        std::fs::write(
            root.join("src/日本語.rs"),
            "fn x() {}\nfn y() {}\nfn z() {}\n",
        )
        .unwrap();
        git_here(root, &["commit", "-qam", "second"]);
        let sha = git_here(root, &["rev-parse", "HEAD"]);

        let files = show_stat(root, &sha).unwrap();
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
    fn show_stat_preserves_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_here(root, &["init", "-q"]);
        git_here(root, &["config", "user.email", "t@t.t"]);
        git_here(root, &["config", "user.name", "t"]);
        std::fs::create_dir(root.join("src")).unwrap();
        let name = std::ffi::OsStr::from_bytes(b"src/\xff\xfe.rs");
        std::fs::write(root.join(name), "fn x() {}\n").unwrap();
        git_here(root, &["add", "."]);
        git_here(root, &["commit", "-qm", "first"]);
        let sha = git_here(root, &["rev-parse", "HEAD"]);

        let files = show_stat(root, &sha).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path.as_os_str().as_bytes(), b"src/\xff\xfe.rs");
        assert_eq!(files[0].added, 1);
    }
}
