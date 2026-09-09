//! Indent scopes, not a tab-stop lattice. Equal-indent text without an opener
//! has no rail; jumps introduce only the actual parent column. Work is performed
//! against a frozen rope by the analysis worker, never during row rendering.
use imbl::Vector;
use ropey::Rope;
use strop_core::id::DisplayColumn;

struct Run {
    first: usize,
    end: usize,
    columns: Vector<DisplayColumn>,
}
#[derive(Default)]
pub struct IndentGuides {
    runs: Vec<Run>,
    prefixes: Vec<usize>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GuideFrame {
    pub first_line: usize,
    pub rows: Vec<Vec<DisplayColumn>>,
}
impl GuideFrame {
    pub fn columns(&self, line: usize) -> &[DisplayColumn] {
        line.checked_sub(self.first_line)
            .and_then(|line| self.rows.get(line))
            .map_or(&[], Vec::as_slice)
    }
}
impl IndentGuides {
    pub fn build(rope: &Rope, tab: usize, cancelled: impl Fn() -> bool) -> Option<Self> {
        let mut runs: Vec<Run> = Vec::new();
        let mut prefixes = Vec::with_capacity(rope.len_lines());
        let mut scopes = Vector::new();
        let mut previous = None;
        let mut blank_start = None;
        let tab = tab.max(1);
        for (row, line) in rope.lines().enumerate() {
            if row % 128 == 0 && cancelled() {
                return None;
            }
            let mut indent = 0usize;
            let mut content = false;
            let mut prefix = 0;
            for ch in line.chars() {
                if prefix % 2048 == 0 && cancelled() {
                    return None;
                }
                match ch {
                    ' ' => indent = indent.saturating_add(1),
                    '\t' => indent = indent.saturating_add(tab - indent % tab),
                    '\n' | '\r' => break,
                    _ => {
                        content = true;
                        break;
                    }
                }
                prefix += ch.len_utf8();
            }
            prefixes.push(prefix);
            if !content {
                blank_start.get_or_insert(row);
                continue;
            }
            while scopes
                .back()
                .is_some_and(|column: &DisplayColumn| column.get() >= indent)
            {
                scopes.pop_back();
            }
            if let Some(parent) = previous.filter(|parent| *parent < indent) {
                let column = DisplayColumn::new(parent);
                if scopes.back() != Some(&column) {
                    scopes.push_back(column);
                }
            }
            previous = Some(indent);
            let first = blank_start.take().unwrap_or(row);
            if let Some(last) = runs.last_mut().filter(|last| last.columns == scopes) {
                last.end = row + 1;
            } else {
                runs.push(Run {
                    first,
                    end: row + 1,
                    columns: scopes.clone(),
                });
            }
        }
        if let Some(first) = blank_start {
            runs.push(Run {
                first,
                end: rope.len_lines(),
                columns: Vector::new(),
            });
        }
        (!cancelled()).then_some(Self { runs, prefixes })
    }
    /// A same-line edit strictly after the first content byte cannot change
    /// nesting. Most ordinary typing reuses the complete guide index.
    pub fn unaffected_by(&self, edit: &strop_core::InputEdit) -> bool {
        edit.start_point.0 == edit.old_end_point.0
            && edit.start_point.0 == edit.new_end_point.0
            && self
                .prefixes
                .get(edit.start_point.0)
                .is_some_and(|prefix| edit.start_point.1 > *prefix)
    }
    pub fn frame(&self, first: usize, end: usize, left: usize, right: usize) -> GuideFrame {
        let mut result = GuideFrame {
            first_line: first,
            rows: Vec::with_capacity(end.saturating_sub(first)),
        };
        let mut run = self.runs.partition_point(|run| run.end <= first);
        for line in first..end {
            while self.runs.get(run).is_some_and(|run| run.end <= line) {
                run += 1;
            }
            result.rows.push(
                self.runs
                    .get(run)
                    .filter(|run| run.first <= line)
                    .map_or_else(Vec::new, |run| {
                        run.columns
                            .iter()
                            .copied()
                            .filter(|column| column.get() >= left && column.get() < right)
                            .collect()
                    }),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_does_not_invent_intermediate_rails() {
        let rope = Rope::from_str("    flat\n    aligned\n            deeper\n\n    back\n");
        let index = IndentGuides::build(&rope, 4, || false).unwrap();
        let frame = index.frame(0, 5, 0, 40);
        assert_eq!(
            frame.rows,
            vec![vec![], vec![], vec![DisplayColumn::new(4)], vec![], vec![]]
        );
    }
    #[test]
    fn nested_tabs_and_dedents_use_display_columns() {
        let rope = Rope::from_str("root\n\tchild\n\t\tinner\n\tpeer\nend\n");
        let frame = IndentGuides::build(&rope, 4, || false)
            .unwrap()
            .frame(0, 5, 0, 8);
        assert_eq!(
            frame.rows,
            vec![
                vec![],
                vec![DisplayColumn::new(0)],
                vec![DisplayColumn::new(0), DisplayColumn::new(4)],
                vec![DisplayColumn::new(0)],
                vec![]
            ]
        );
    }
    #[test]
    fn blank_rows_follow_the_surrounding_scope_not_the_previous_indent() {
        let rope = Rope::from_str("root\n\n    child\n        nested\n\n    peer\n\nend\n");
        let frame = IndentGuides::build(&rope, 4, || false)
            .unwrap()
            .frame(0, 8, 0, 20);
        assert_eq!(frame.columns(1), &[DisplayColumn::new(0)]);
        assert_eq!(frame.columns(4), &[DisplayColumn::new(0)]);
        assert!(frame.columns(6).is_empty());
    }
}
