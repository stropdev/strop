//! The commit log and a file's history (`Space g l`, `Space g h`; plans
//! 0101 §2 and 0100 §7): typed rows, parsed incrementally so a long walk
//! can be shown while git is still printing it.
//!
//! Each commit row is asked of git as one line: the graph runes, a record
//! separator, then the fields separated by unit separators, the subject
//! last so nothing in it can shift a field. Lines without the separator
//! are the graph's connector rows (`|\`, `|/`).

use std::ffi::OsString;
use std::path::Path;

use strop_core::worker::CancelToken;

use crate::exec::GitExec;

/// What a history read asks git for, after the graph: full and short
/// sha, author, author date (unix seconds), ref names, subject.
const FIELDS: &str = "%H%x1f%h%x1f%an%x1f%at%x1f%D%x1f%s";
/// Starts the fields of a commit row; never part of a graph.
const RECORD: char = '\x1e';
const FIELD: char = '\x1f';

/// One commit as the log shows it (0100 §7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogCommit {
    /// The full sha.
    pub sha: String,
    /// The abbreviated sha git chose for this repository (`%h`).
    pub short: String,
    /// Ref names pointing here, as git decorates them: `HEAD -> main`,
    /// `tag: v1.0`, `origin/main`. Empty for most commits.
    pub refs: Vec<String>,
    pub subject: String,
    pub author: String,
    /// Author date, unix seconds: the renderer words the log's age from
    /// it against the editor's clock (0100 §4.3).
    pub time: i64,
}

/// One row of the log: a commit with the graph runes ahead of it, or a
/// graph-only connector row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogRow {
    /// The graph column of this row (empty without `--graph`, as in a
    /// line-range history). For a connector row, the whole row.
    pub graph: String,
    pub commit: Option<LogCommit>,
}

impl LogRow {
    /// The full sha, when the row names a commit.
    pub fn sha(&self) -> Option<&str> {
        self.commit.as_ref().map(|commit| commit.sha.as_str())
    }
}

/// Splits git's history output into rows as the bytes arrive, whatever
/// the chunk boundaries: a row is parsed once its newline is in.
#[derive(Debug, Default)]
pub struct LogRecords {
    pending: Vec<u8>,
    /// Line-range history prints patches between its commit rows; only
    /// lines that start with a record are rows there.
    ranged: bool,
}

impl LogRecords {
    pub fn new(ranged: bool) -> Self {
        Self {
            pending: Vec::new(),
            ranged,
        }
    }

    /// Feed bytes; returns the rows they completed.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<LogRow> {
        self.pending.extend_from_slice(bytes);
        let Some(last) = self.pending.iter().rposition(|byte| *byte == b'\n') else {
            return Vec::new();
        };
        let rest = self.pending.split_off(last + 1);
        let complete = std::mem::replace(&mut self.pending, rest);
        complete
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .filter_map(|line| self.row(line))
            .collect()
    }

    /// The end of the stream: a last row without its newline.
    pub fn finish(mut self) -> Option<LogRow> {
        let line = std::mem::take(&mut self.pending);
        (!line.is_empty()).then(|| self.row(&line)).flatten()
    }

    fn row(&self, line: &[u8]) -> Option<LogRow> {
        let line = String::from_utf8_lossy(line);
        let Some((graph, fields)) = line.split_once(RECORD) else {
            return (!self.ranged).then(|| LogRow {
                graph: line.into_owned(),
                commit: None,
            });
        };
        if self.ranged && !graph.is_empty() {
            return None; // patch text that happens to hold a separator
        }
        let mut field = fields.splitn(6, FIELD);
        let mut next = || field.next().unwrap_or_default().to_string();
        let (sha, short, author, time, refs, subject) =
            (next(), next(), next(), next(), next(), next());
        let refs = refs
            .split(", ")
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        Some(LogRow {
            graph: graph.to_string(),
            commit: Some(LogCommit {
                sha,
                short,
                refs,
                subject,
                author,
                time: time.parse().unwrap_or(0),
            }),
        })
    }
}

/// Where a history read runs and how it reports.
pub struct HistoryRead<'a> {
    pub exec: &'a GitExec<'a>,
    pub cancel: &'a CancelToken,
    /// The most commits to list.
    pub max: usize,
    /// A file's history instead of the whole log.
    pub file: Option<&'a Path>,
    /// A line range of that file (`git log -L`).
    pub range: Option<(usize, usize)>,
    /// strop's private commit-graph for this repository (0101 §2.1),
    /// named to git as an alternate object directory.
    pub graph: Option<&'a Path>,
}

/// The whole log, a file's history or a line range's, streamed: `sink`
/// gets the rows in git's order as they are parsed.
pub fn read_history(
    read: &HistoryRead<'_>,
    mut sink: impl FnMut(Vec<LogRow>),
) -> Result<(), String> {
    let ranged = read.range.is_some();
    // file operands pass as OsStr: a non-UTF8 tracked filename must
    // reach git byte-for-byte, not via a lossy display() rendering
    let mut argv: Vec<OsString> = vec![
        "log".into(),
        format!("--format={RECORD}{FIELDS}").into(),
        "-n".into(),
        read.max.to_string().into(),
    ];
    match (read.file, read.range) {
        (Some(f), Some((a, b))) => {
            // -L embeds the path in one argument; compose the OsString
            // instead of formatting through display()
            let mut spec = OsString::from(format!("-L{a},{b}:"));
            spec.push(f);
            argv.push(spec);
        }
        (Some(f), None) => {
            argv.push("--graph".into());
            argv.push("--".into());
            argv.push(f.as_os_str().into());
        }
        (None, None) => argv.push("--graph".into()),
        (None, Some(_)) => return Err("-L needs a file".into()),
    }
    let env: Vec<(&str, &std::ffi::OsStr)> = read
        .graph
        .map(|graph| ("GIT_ALTERNATE_OBJECT_DIRECTORIES", graph.as_os_str()))
        .into_iter()
        .collect();
    let mut records = LogRecords::new(ranged);
    read.exec
        .stream_records("git log", &argv, &env, read.cancel, |bytes| {
            let rows = records.push(bytes);
            if !rows.is_empty() {
                sink(rows);
            }
        })?;
    if let Some(row) = records.finish() {
        sink(vec![row]);
    }
    Ok(())
}

/// [`read_history`] collected into one list.
pub fn history(read: &HistoryRead<'_>) -> Result<Vec<LogRow>, String> {
    let mut rows = Vec::new();
    read_history(read, |page| rows.extend(page))?;
    Ok(rows)
}

#[cfg(test)]
mod tests;
