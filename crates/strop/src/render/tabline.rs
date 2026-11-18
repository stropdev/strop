//! The tab line (plans/0113 §5): row 0 of the screen, only while there are
//! two or more tab pages. A band on the modeline's plane (`bg.surface`);
//! the current label alone sits on the text background, the label of the
//! panes below it (0103 §4.3). A label is ` N name[ +k][ ●] `: the tab
//! page's number, the last component of what its current pane shows, how
//! many more panes it holds, and whether one of them shows unsaved
//! changes. No accent, no close mark, nothing on the right.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use strop_engine::editor::tabs;

use super::clip::{clip_end, width};
use super::style::Tokens;
use crate::editor::Editor;

/// Names are cut to at most this many cells before anything else gives.
const LONGEST: usize = 24;
/// …and never below this many.
const SHORTEST: usize = 6;

/// One tab page as the row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tab {
    pub number: String,
    pub name: String,
    /// More panes than the one named.
    pub more: usize,
    pub unsaved: bool,
}

impl Tab {
    /// ` +k` and ` ●`, in cells.
    fn marks(&self) -> usize {
        let more = if self.more > 0 {
            2 + self.more.to_string().len()
        } else {
            0
        };
        more + if self.unsaved { 2 } else { 0 }
    }

    /// The label's cells with the name cut to `cap` (`None`: no name).
    fn cells(&self, cap: Option<usize>) -> usize {
        let name = cap.map_or(0, |cap| 1 + width(&self.name).min(cap));
        1 + self.number.len() + name + self.marks() + 1
    }
}

/// What fits (rule 6): the tab pages shown, the cap on names, and whether
/// every label but the current one has dropped its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Fit {
    pub first: usize,
    /// One past the last tab page shown.
    pub end: usize,
    pub cap: usize,
    pub short: bool,
}

impl Fit {
    /// The name cap of tab page `index`, `None` when it shows no name.
    fn cap(&self, index: usize, current: usize) -> Option<usize> {
        (!self.short || index == current).then_some(self.cap)
    }

    fn cells(&self, tabs: &[Tab], current: usize) -> usize {
        let labels: usize = (self.first..self.end)
            .map(|index| tabs[index].cells(self.cap(index, current)))
            .sum();
        labels + usize::from(self.first > 0) + usize::from(self.end < tabs.len())
    }
}

/// Rule 6: every label whole; else names cut, 24 cells down to 6; else
/// the others short and the current name cut the same way; else a run of
/// short labels around the current one, `‹` `›` where tab pages go on.
pub(super) fn fit(tabs: &[Tab], current: usize, columns: usize) -> Fit {
    let caps = std::iter::once(usize::MAX).chain((SHORTEST..=LONGEST).rev());
    for short in [false, true] {
        for cap in caps.clone() {
            let fit = Fit {
                first: 0,
                end: tabs.len(),
                cap,
                short,
            };
            if fit.cells(tabs, current) <= columns {
                return fit;
            }
        }
    }
    // the current label first, then its right and left neighbours in turn
    let mut fit = Fit {
        first: current,
        end: current + 1,
        cap: SHORTEST,
        short: true,
    };
    loop {
        let mut grew = false;
        let right = Fit {
            end: fit.end + 1,
            ..fit
        };
        if fit.end < tabs.len() && right.cells(tabs, current) <= columns {
            fit = right;
            grew = true;
        }
        if fit.first > 0 {
            let left = Fit {
                first: fit.first - 1,
                ..fit
            };
            if left.cells(tabs, current) <= columns {
                fit = left;
                grew = true;
            }
        }
        if !grew {
            return fit;
        }
    }
}

/// What a run of cells on the row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    /// The number, `+k` and the blanks around them.
    Plain,
    Name,
    Unsaved,
    /// `‹` or `›`.
    More,
}

/// The row as runs of text: `(text, part, on the current label)`.
pub(super) fn runs(tabs: &[Tab], current: usize, columns: usize) -> Vec<(String, Part, bool)> {
    let fit = fit(tabs, current, columns);
    let mut runs = Vec::new();
    if fit.first > 0 {
        runs.push(("‹".to_owned(), Part::More, false));
    }
    for (index, tab) in tabs.iter().enumerate().take(fit.end).skip(fit.first) {
        let here = index == current;
        let mut plain = format!(" {}", tab.number);
        if let Some(cap) = fit.cap(index, current) {
            plain.push(' ');
            runs.push((plain, Part::Plain, here));
            runs.push((clip_end(&tab.name, cap).into_owned(), Part::Name, here));
            plain = String::new();
        }
        if tab.more > 0 {
            plain.push_str(&format!(" +{}", tab.more));
        }
        if tab.unsaved {
            runs.push((plain, Part::Plain, here));
            runs.push((" ●".to_owned(), Part::Unsaved, here));
            plain = String::new();
        }
        plain.push(' ');
        runs.push((plain, Part::Plain, here));
    }
    if fit.end < tabs.len() {
        runs.push(("›".to_owned(), Part::More, false));
    }
    runs
}

/// The last `/` component of a pane title's name; a folder keeps its `/`.
fn last_component(label: &str) -> &str {
    let trimmed = label.strip_suffix('/').unwrap_or(label);
    trimmed
        .rfind('/')
        .map_or(label, |slash| &label[slash + 1..])
}

fn tab(editor: &Editor, index: usize) -> Option<Tab> {
    let summary = tabs::summary(editor, index)?;
    let document = editor.document(summary.document)?;
    let root = document
        .buf
        .path
        .as_deref()
        .and_then(|path| editor.workspace_root_path(&strop_workspace::Filesystem::Local, path))
        .unwrap_or(editor.cwd());
    Some(Tab {
        number: (index + 1).to_string(),
        name: last_component(&document.label(root)).to_owned(),
        more: summary.panes - 1,
        unsaved: summary.unsaved,
    })
}

pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) {
    let count = tabs::count(editor);
    if count < 2 || area.width == 0 || area.height == 0 {
        return;
    }
    let tabs: Vec<Tab> = (0..count).filter_map(|index| tab(editor, index)).collect();
    let current = tabs::current(editor);
    let spans: Vec<Span> = runs(&tabs, current, usize::from(area.width))
        .into_iter()
        .map(|(text, part, here)| {
            let bg = if here { t.base } else { t.surface };
            let ink = if here { t.text } else { t.muted };
            let style = match part {
                Part::Plain => Style::default().fg(ink),
                Part::Name => Style::default().fg(ink).add_modifier(Modifier::BOLD),
                Part::Unsaved => Style::default().fg(t.warning),
                Part::More => Style::default().fg(t.faint),
            };
            Span::styled(text, style.bg(bg))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
        Rect { height: 1, ..area },
    );
}
