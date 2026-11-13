//! The log's buffer text (0100 §4.3 step B): `graph sha  (refs) subject`,
//! the graph and the sha padded so the shas and subjects line up. Author
//! and age are not in the text: they are the row's fact, drawn by the
//! renderer from the typed row (0100 §3.8), so a yanked row is the
//! commit's sha and subject.

use strop_git::log::LogRow;

/// The widths the leading columns of a log's rows are padded to: the
/// widest graph and the widest abbreviated sha among the rows loaded.
/// Pages only widen them; when a page does, the rows above are rebuilt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogColumns {
    pub graph: usize,
    pub sha: usize,
}

impl LogColumns {
    /// Widen to fit `rows`' commits. Graph runes and shas are ASCII, so
    /// their byte length is their width.
    pub(crate) fn fit(&mut self, rows: &[LogRow]) {
        for row in rows {
            if let Some(commit) = &row.commit {
                self.graph = self.graph.max(row.graph.len());
                self.sha = self.sha.max(commit.short.len());
            }
        }
    }
}

/// One row's buffer text. A connector row (`|\`) is its graph alone.
pub fn row_text(row: &LogRow, columns: LogColumns) -> String {
    let Some(commit) = &row.commit else {
        return row.graph.clone();
    };
    let mut text = format!(
        "{:graph$}{:sha$}  ",
        row.graph,
        commit.short,
        graph = columns.graph,
        sha = columns.sha
    );
    if !commit.refs.is_empty() {
        text.push('(');
        text.push_str(&commit.refs.join(", "));
        text.push_str(") ");
    }
    text.push_str(&commit.subject);
    text
}

/// The rows' text, one line each.
pub(crate) fn text(rows: &[LogRow], columns: LogColumns) -> String {
    let mut text = String::new();
    for row in rows {
        text.push_str(&row_text(row, columns));
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_git::log::LogCommit;

    fn commit(graph: &str, short: &str, refs: &[&str], subject: &str) -> LogRow {
        LogRow {
            graph: graph.into(),
            commit: Some(LogCommit {
                sha: format!("{short}000"),
                short: short.into(),
                refs: refs.iter().map(|r| r.to_string()).collect(),
                subject: subject.into(),
                author: "Ada".into(),
                time: 0,
            }),
        }
    }

    /// The shas and the subjects line up whatever the graph's width;
    /// the refs ride in front of the subject; author and age are not text.
    #[test]
    fn shas_and_subjects_line_up() {
        let rows = vec![
            commit("*   ", "0d21f9e", &["HEAD -> main"], "merge"),
            LogRow {
                graph: "|\\".into(),
                commit: None,
            },
            commit("| * ", "e8c10345", &[], "side"),
            commit("* ", "8cc6dd6", &["tag: v1", "origin/main"], "base"),
        ];
        let mut columns = LogColumns::default();
        columns.fit(&rows);
        assert_eq!(
            text(&rows, columns),
            "*   0d21f9e   (HEAD -> main) merge\n\
             |\\\n\
             | * e8c10345  side\n\
             *   8cc6dd6   (tag: v1, origin/main) base\n"
        );
    }

    /// A line-range history has no graph: the sha leads.
    #[test]
    fn no_graph_no_padding() {
        let rows = vec![commit("", "abc1234", &[], "fix")];
        let mut columns = LogColumns::default();
        columns.fit(&rows);
        assert_eq!(text(&rows, columns), "abc1234  fix\n");
    }
}
