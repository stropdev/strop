//! Git memory (M3, 0001 pillar 3.2/3.3): log graph, blame, permalinks.
//! Reads via shell `git` (matches user config; not hot-path), permalinks
//! via libgit2 config (no spawn).

use std::path::{Path, PathBuf};

use crate::Repo;

/// One log line from `git log --graph`, with the commit hash extracted.
#[derive(Debug, Clone)]
pub struct LogRow {
    /// The rendered graph+summary line (what the buffer shows).
    pub text: String,
    /// Full SHA when the line names a commit (graph-only lines: None).
    pub sha: Option<String>,
}

/// `git log --graph` for the browser. Shells out — the log is not a
/// per-keystroke path (0001 §3). Caller decides threading.
pub fn log_graph(workdir: &Path, max: usize, file: Option<&Path>) -> Result<Vec<LogRow>, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args([
        "-C",
        &workdir.display().to_string(),
        "log",
        "--graph",
        "--format=%h %an · %ar · %s%x00%H",
        "-n",
        &max.to_string(),
    ]);
    if let Some(f) = file {
        cmd.arg("--").arg(f);
    }
    let out = cmd.output().map_err(|e| format!("spawn git log: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .map(|line| {
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
#[derive(Debug, Clone)]
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
        .args([
            "-C",
            &workdir.display().to_string(),
            "blame",
            "--line-porcelain",
            "-L",
            &format!("{line},{line}"),
            "--",
            &rel.display().to_string(),
        ])
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

/// Files changed by a commit: `path | +N -M` rows for the dive view.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub added: usize,
    pub deleted: usize,
}

pub fn show_stat(workdir: &Path, sha: &str) -> Result<Vec<ChangedFile>, String> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            &workdir.display().to_string(),
            "show",
            "--numstat",
            "--format=",
            sha,
        ])
        .output()
        .map_err(|e| format!("spawn git show: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let added = parts.next()?.parse().ok()?;
            let deleted = parts.next()?.parse().ok()?;
            Some(ChangedFile {
                path: PathBuf::from(parts.next()?),
                added,
                deleted,
            })
        })
        .collect())
}

/// Unified diff for one file at a commit (the delta view).
pub fn show_file_delta(workdir: &Path, sha: &str, file: &Path) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            &workdir.display().to_string(),
            "show",
            "--format=",
            "--patch",
            sha,
            "--",
            &file.display().to_string(),
        ])
        .output()
        .map_err(|e| format!("spawn git show: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ---- permalinks ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    GitHub,
    GitLab,
    Bitbucket,
    Gitea,
    /// Unknown host: emit whatever HTTPS we can normalize to.
    Other,
}

pub struct Remote {
    pub host: Host,
    pub owner_repo: String, // "org/repo"
    pub base: String,       // "https://github.com"
}

/// Normalize a remote URL (SSH or HTTPS) to a web base. Priority
/// upstream > origin > rest is the caller's job (0001 pillar 3.3).
pub fn normalize_remote(url: &str) -> Option<Remote> {
    let url = url.trim().trim_end_matches(".git");
    let (base, path) = if let Some(rest) = url.strip_prefix("git@") {
        // git@host:org/repo
        let (host, path) = rest.split_once(':')?;
        (format!("https://{host}"), path.to_string())
    } else if let Some(rest) = url.strip_prefix("ssh://git@") {
        // ssh://git@host/org/repo
        let rest = rest.split('/').collect::<Vec<_>>();
        let host = rest.first()?;
        (format!("https://{host}"), rest[1..].join("/"))
    } else if url.starts_with("https://") || url.starts_with("http://") {
        let stripped = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let (host, path) = stripped.split_once('/')?;
        (format!("https://{host}"), path.to_string())
    } else {
        return None;
    };
    let host = match base.as_str() {
        "https://github.com" => Host::GitHub,
        "https://gitlab.com" => Host::GitLab,
        "https://bitbucket.org" => Host::Bitbucket,
        b if b.contains("gitea") => Host::Gitea,
        _ => Host::Other,
    };
    Some(Remote {
        host,
        owner_repo: path,
        base,
    })
}

/// Pick the permalink remote: upstream > origin > first remaining.
pub fn pick_remote(repo: &Repo) -> Option<Remote> {
    let remotes = repo.remotes();
    for name in ["upstream", "origin"] {
        if let Some(url) = remotes.iter().find(|(n, _)| n == name).map(|(_, u)| u) {
            if let Some(r) = normalize_remote(url) {
                return Some(r);
            }
        }
    }
    remotes.iter().find_map(|(_, u)| normalize_remote(u))
}

/// Build the immutable permalink for a file at 1-based lines. Branch is
/// always resolved to a commit SHA (0001 pillar 3.3).
pub fn permalink(repo: &Repo, rel: &Path, start_line: usize, end_line: usize) -> Option<String> {
    let remote = pick_remote(repo)?;
    let sha = repo.head_sha()?;
    let frag = if start_line == end_line {
        format!("#L{start_line}")
    } else {
        format!("#L{start_line}-L{end_line}")
    };
    Some(format!(
        "{}/{}/blob/{}/{}{frag}",
        remote.base,
        remote.owner_repo,
        sha,
        rel.display()
    ))
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

    #[test]
    fn normalizes_ssh_and_https() {
        let r = normalize_remote("git@github.com:stropdev/strop.git").unwrap();
        assert_eq!(
            (r.base.as_str(), r.owner_repo.as_str()),
            ("https://github.com", "stropdev/strop")
        );
        assert_eq!(r.host, Host::GitHub);
        let r = normalize_remote("https://gitlab.com/org/proj").unwrap();
        assert_eq!(r.host, Host::GitLab);
        assert_eq!(r.owner_repo, "org/proj");
        let r = normalize_remote("ssh://git@bitbucket.org/team/repo.git").unwrap();
        assert_eq!(r.host, Host::Bitbucket);
        assert!(normalize_remote("not a url").is_none());
    }
}
