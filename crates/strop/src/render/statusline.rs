//! The modeline (0032): one quiet row at the terminal's bottom edge.
//!
//! The hierarchy borrows from the observed OMP footer — a compact mode
//! accent, a clear filename over muted directory context, and quiet,
//! separated Git/diagnostic/position groups — in strop's own palette
//! and without font-dependent glyphs. Every width here is a display
//! cell (`text`), never a character: under pressure the row yields the
//! directory first, then the Git context, then the optional signals,
//! and only then clips the filename and the transient status — the
//! mode accent and the position are never pushed off the row. Every
//! gathered string passes the shared clipping helpers, so a control
//! byte can never reach the terminal as protocol.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use strop_lsp::Severity;

use crate::editor::Editor;

use super::text::{clip_end, clip_start, width};
use super::{cmd_card_active, mode_color, severity_color, ACCENT, BASE, MUTED, TEXT};

#[cfg(test)]
mod tests;

/// Quiet group separator: the palette's punctuation value, dimmer than
/// muted text so groups read as groups (0032's "quiet separators").
const QUIET: Color = Color::Rgb(0x56, 0x5b, 0x6e);
/// Held-back amber: read-only files and staged changes — real state,
/// softer than the worktree's ACCENT.
const HELD: Color = Color::Rgb(0xe0, 0xaf, 0x68);
const BACKGROUND: Color = Color::Rgb(0x20, 0x22, 0x2b);

/// A separator costs three cells (" │ ", padding included).
const SEP: &str = " │ ";
const SEP_W: usize = 3;
/// The halves never jam together while a spare cell exists.
const GAP: usize = 1;
/// Smallest fragment still worth its cells (a directory hint, a name,
/// a position in the degenerate layout).
const MIN_GROUP: usize = 4;
/// A transient status shorter than this is noise, not information.
const MIN_STATUS: usize = 3;

pub(super) fn render(editor: &Editor, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return; // a 0-sized resize must not underflow (0027 §2)
    }
    let cells = area.width as usize;
    let mut row = Modeline::gather(editor);
    row.fit(cells);
    let rect = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Line::from(row.spans(cells))).style(Style::default().bg(BACKGROUND)),
        rect,
    );
}

/// Everything the row can show. Strings are gathered (and sanitized)
/// once; [`Modeline::fit`] then narrows the model in place — dropping
/// or clipping fields — until it fits the terminal's cell width.
/// Width fitting borrows the gathered fields; spans are allocated only
/// for the final row, not for every degradation step.
struct Modeline {
    chip: String, // " NORMAL " — the label with its padding
    accent: Color,
    git_context: String,         // branch or historical commit identity
    worktree_dirty: bool,        // "*" — unstaged or untracked changes
    staged_mark: bool,           // "+" — staged changes
    dir: String,                 // muted context, trailing separator kept
    name: String,                // the part that must survive truncation
    dirty: bool,                 // " ●" after the name
    readonly: bool,              // " [RO]"
    multicursor: Option<String>, // "3×", without its leading pad
    status: String,              // the transient right-side text
    errors: usize,
    warnings: usize,
    show_diag: bool,
    line: usize,
    col: usize,
    percent: Option<usize>,
    bare: bool,       // degenerate layout: mode accent · position only
    bare_chip: usize, // cells the chip may occupy in that layout
}

impl Modeline {
    fn gather(editor: &Editor) -> Self {
        let buf = editor.buf();
        let (dir, name) = file_display(editor);
        let line = buf.line_of(editor.head()) + 1;
        let col = buf.col_of(editor.head()) + 1;
        // The percentage's denominator is content lines: a trailing
        // newline's phantom row is not somewhere you can be 75% down
        // of — the last content row reports 100 (0032).
        let content = buf.last_content_line() + 1;
        let percent = if content <= 1 {
            100
        } else {
            (line * 100 / content).min(100)
        };
        let (errors, warnings) = editor.diag_counts(editor.current());
        let cursors = editor.sels().count();
        let commit = historical_commit(editor);
        let git_context = match commit {
            Some(sha) => format!("@{}", sha.get(..8).unwrap_or(sha)),
            None if editor.remote_file().is_some() => editor
                .remote_file()
                .map(|file| format!("ssh:{}", file.endpoint().host()))
                .unwrap_or_default(),
            None => editor
                .git
                .as_ref()
                .and_then(|git| git.head_branch.clone())
                .unwrap_or_default(),
        };
        Self {
            chip: format!(" {} ", editor.mode.chip()),
            accent: mode_color(editor.mode),
            git_context: printable(git_context),
            worktree_dirty: commit.is_none()
                && (!editor.hunks.is_empty() || editor.hunks_untracked),
            staged_mark: commit.is_none() && !editor.staged_hunks.is_empty(),
            dir: printable(dir),
            name: printable(name),
            dirty: buf.dirty,
            readonly: buf.readonly,
            multicursor: (cursors > 1).then(|| format!("{cursors}×")),
            status: printable(transient(editor)),
            errors,
            warnings,
            show_diag: errors > 0 || warnings > 0,
            line,
            col,
            percent: Some(percent),
            bare: false,
            bare_chip: 0,
        }
    }

    /// Narrow the row until it fits `cells`. Each step frees the least
    /// essential thing still on screen; the first step that makes it
    /// fit wins, so a wide terminal loses nothing.
    fn fit(&mut self, cells: usize) {
        if self.used() <= cells {
            return;
        }
        self.staged_mark = false;
        if self.used() <= cells {
            return;
        }
        // The directory yields before anything else — shrunk towards
        // its tail (the parent closest to the file is the context
        // worth keeping), but never below a smallest useful fragment.
        // The filename is never the thing that gives way.
        let overflow = self.used() - cells;
        let budget = MIN_GROUP.max(width(&self.dir).saturating_sub(overflow));
        if width(&self.dir) > budget {
            self.dir = clip_start(&self.dir, budget).into_owned();
        }
        if self.used() <= cells {
            return;
        }
        self.dir.clear();
        if self.used() <= cells {
            return;
        }
        self.git_context.clear();
        if self.used() <= cells {
            return;
        }
        self.multicursor = None;
        if self.used() <= cells {
            return;
        }
        self.readonly = false;
        if self.used() <= cells {
            return;
        }
        self.percent = None;
        if self.used() <= cells {
            return;
        }
        self.show_diag = false;
        if self.used() <= cells {
            return;
        }
        self.elastic(cells);
    }

    /// The incompressible ends are the mode accent and the position;
    /// everything optional is already gone. What is left is shared
    /// between the filename and the transient status — the filename
    /// first, but capped at two thirds so a huge name cannot starve
    /// live activity (0032: a long path must not crowd out the
    /// status).
    fn elastic(&mut self, cells: usize) {
        let chip_w = 1 + width(&self.chip);
        let pos_w = self.position_width();
        if !self.status.is_empty() {
            let Some(room) = cells.checked_sub(chip_w + SEP_W + GAP + SEP_W + pos_w) else {
                self.status.clear(); // its separator is the next cell to free
                return self.elastic(cells);
            };
            let file_budget = self.file_w().min(room).min((room * 2 / 3).max(MIN_GROUP));
            self.fit_file(file_budget);
            let status_budget = room - file_budget;
            if status_budget >= MIN_STATUS {
                self.status = clip_end(&self.status, status_budget).into_owned();
            } else {
                self.status.clear();
            }
            return;
        }
        let Some(room) = cells.checked_sub(chip_w + SEP_W + GAP + pos_w) else {
            return self.ultra(cells);
        };
        self.fit_file(room);
    }

    /// Spend `budget` cells on the file group: the filename first
    /// (clipped from the end, whole graphemes only), then whichever
    /// signals still fit behind it.
    fn fit_file(&mut self, budget: usize) {
        if width(&self.name) > budget {
            self.name = clip_end(&self.name, budget).into_owned();
        }
        let mut room = budget.saturating_sub(width(&self.name));
        if self.dirty && room >= 2 {
            room -= 2;
        } else {
            self.dirty = false;
        }
        if self.readonly && room >= 5 {
            room -= 5;
        } else {
            self.readonly = false;
        }
        if let Some(mark) = self.multicursor.as_ref() {
            if room < 1 + width(mark) {
                self.multicursor = None;
            }
        }
    }

    /// Degenerate widths: not even the accent, a separator and the
    /// position fit. Show the accent and the position alone, splitting
    /// the row between them — never an empty row.
    fn ultra(&mut self, cells: usize) {
        self.git_context.clear();
        self.dir.clear();
        self.name.clear();
        self.status.clear();
        self.dirty = false;
        self.readonly = false;
        self.multicursor = None;
        self.percent = None;
        self.show_diag = false;
        self.bare = true;
        let pos_budget = self.position_width().min((cells / 2).max(MIN_GROUP));
        self.bare_chip = cells.saturating_sub(pos_budget);
    }

    /// `line:col` and, while there is room, the content percentage —
    /// units unchanged from the 0.15 modeline (1-based line, 1-based
    /// byte column).
    fn position(&self) -> String {
        match self.percent {
            Some(percent) => format!("{}:{} {}%", self.line, self.col, percent),
            None => format!("{}:{}", self.line, self.col),
        }
    }

    fn position_width(&self) -> usize {
        digits(self.line)
            + 1
            + digits(self.col)
            + self.percent.map_or(0, |percent| 2 + digits(percent))
    }

    /// Measure the visible groups without building or allocating spans.
    fn used(&self) -> usize {
        let mut left = 1 + width(&self.chip);
        if !self.git_context.is_empty() {
            left += SEP_W
                + width(&self.git_context)
                + usize::from(self.worktree_dirty)
                + usize::from(self.staged_mark);
        }
        if !self.dir.is_empty() || !self.name.is_empty() {
            left += SEP_W + self.file_w();
        }
        let mut right = self.position_width();
        if !self.status.is_empty() {
            right += width(&self.status) + SEP_W;
        }
        if self.show_diag {
            right += SEP_W
                + if self.errors > 0 {
                    1 + digits(self.errors)
                } else {
                    0
                }
                + if self.warnings > 0 {
                    1 + digits(self.warnings)
                } else {
                    0
                }
                + usize::from(self.errors > 0 && self.warnings > 0);
        }
        left + GAP + right
    }

    fn file_w(&self) -> usize {
        width(&self.dir)
            + width(&self.name)
            + 2 * usize::from(self.dirty)
            + 5 * usize::from(self.readonly)
            + self.multicursor.as_ref().map_or(0, |mark| 1 + width(mark))
    }

    fn file_spans(&self) -> Vec<Span<'_>> {
        let mut spans = Vec::new();
        if !self.dir.is_empty() {
            spans.push(Span::styled(self.dir.as_str(), Style::default().fg(MUTED)));
        }
        if !self.name.is_empty() {
            spans.push(Span::styled(self.name.as_str(), Style::default().fg(TEXT)));
        }
        if self.dirty {
            spans.push(Span::styled(" ●", Style::default().fg(ACCENT)));
        }
        if self.readonly {
            spans.push(Span::styled(" [RO]", Style::default().fg(HELD)));
        }
        if let Some(mark) = &self.multicursor {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                mark.as_str(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        }
        spans
    }

    /// Build the visible halves once, after all width decisions.
    fn halves(&self) -> (Vec<Span<'_>>, Vec<Span<'_>>) {
        let quiet = Style::default().fg(QUIET);
        let mut left = vec![
            Span::styled("▌", Style::default().fg(self.accent)),
            Span::styled(
                self.chip.as_str(),
                Style::default()
                    .fg(BASE)
                    .bg(self.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !self.git_context.is_empty() {
            left.push(Span::styled(SEP, quiet));
            left.push(Span::styled(
                self.git_context.as_str(),
                Style::default().fg(MUTED),
            ));
            if self.worktree_dirty {
                left.push(Span::styled("*", Style::default().fg(ACCENT)));
            }
            if self.staged_mark {
                left.push(Span::styled("+", Style::default().fg(HELD)));
            }
        }
        if !self.dir.is_empty() || !self.name.is_empty() {
            left.push(Span::styled(SEP, quiet));
            left.extend(self.file_spans());
        }
        let mut right = Vec::new();
        if !self.status.is_empty() {
            right.push(Span::styled(
                self.status.as_str(),
                Style::default().fg(ACCENT),
            ));
        }
        if self.show_diag {
            if !right.is_empty() {
                right.push(Span::styled(SEP, quiet));
            }
            if self.errors > 0 {
                right.push(Span::styled(
                    format!("●{}", self.errors),
                    Style::default().fg(severity_color(Severity::Error)),
                ));
            }
            if self.warnings > 0 {
                if self.errors > 0 {
                    right.push(Span::raw(" "));
                }
                right.push(Span::styled(
                    format!("●{}", self.warnings),
                    Style::default().fg(severity_color(Severity::Warning)),
                ));
            }
        }
        if !right.is_empty() {
            right.push(Span::styled(SEP, quiet));
        }
        right.push(Span::styled(self.position(), Style::default().fg(MUTED)));
        (left, right)
    }

    /// The finished row: halves joined by a flexible gap that never
    /// closes while both halves are on screen.
    fn spans(&self, cells: usize) -> Vec<Span<'_>> {
        if self.bare {
            return self.bare_spans(cells);
        }
        let (left, right) = self.halves();
        // the fit invariant already reserved the gap: l + GAP + r
        // fits, so this pad is at least GAP cells wide
        let pad = cells.saturating_sub(span_width(&left) + span_width(&right));
        let mut spans = left;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right);
        spans
    }

    fn bare_spans(&self, cells: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        if self.bare_chip >= 1 {
            spans.push(Span::styled("▌", Style::default().fg(self.accent)));
        }
        if self.bare_chip >= 2 {
            spans.push(Span::styled(
                clip_end(&self.chip, self.bare_chip - 1).into_owned(),
                Style::default()
                    .fg(BASE)
                    .bg(self.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            clip_end(&self.position(), cells.saturating_sub(self.bare_chip)).into_owned(),
            Style::default().fg(MUTED),
        ));
        spans
    }
}

fn span_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| width(&span.content)).sum()
}

fn digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}

fn printable(text: String) -> String {
    strop_core::layout::printable_text(text).into_owned()
}

/// A historical view names its captured commit, not today's worktree branch.
fn historical_commit(editor: &Editor) -> Option<&str> {
    match editor.surface() {
        Some(crate::editor::Surface::ChangedFiles { sha, .. }) => Some(sha),
        Some(crate::editor::Surface::Diff {
            commit: Some(commit),
            ..
        }) => Some(&commit.sha),
        _ => None,
    }
}

/// The file group's text: the workspace-relative native path, split so
/// the filename — the part that must survive truncation — can be
/// emphasized over its directory. `io` opens paths under the editor's
/// cwd, so `strip_prefix` relativizes; anything outside keeps its
/// absolute form. Virtual buffers keep their display name; a pathless,
/// nameless buffer is the scratch.
fn file_display(editor: &Editor) -> (String, String) {
    if let Some(file) = editor.remote_file() {
        let directory = file.path().parent().map_or_else(String::new, |parent| {
            let mut directory = parent.display().to_string();
            if !directory.ends_with('/') {
                directory.push('/');
            }
            directory
        });
        let mut name = file
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(source) = editor.cur().remote_metadata() {
            if !source.window.is_complete() || editor.remote_following(editor.current()) {
                let window = source.window;
                name = format!(
                    "[{}{}..{}/{} B] {name}",
                    if editor.remote_following(editor.current()) {
                        "follow "
                    } else {
                        ""
                    },
                    window.start().get(),
                    window.start().get() + window.length().get(),
                    window.file_size().get()
                );
            }
        }
        return (directory, name);
    }
    let buf = editor.buf();
    let path = match editor.surface() {
        Some(crate::editor::Surface::Diff {
            commit: Some(commit),
            ..
        }) => Some(commit.current.as_path()),
        _ => buf.path.as_deref(),
    };
    let Some(path) = path else {
        return (
            String::new(),
            buf.name.as_deref().unwrap_or("[scratch]").to_owned(),
        );
    };
    let relative = path
        .strip_prefix(&editor.cwd)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(path);
    let Some(name) = relative.file_name() else {
        return (String::new(), relative.to_string_lossy().into_owned());
    };
    let mut directory = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(String::new, |parent| parent.to_string_lossy().into_owned());
    if !directory.is_empty() {
        directory.push(std::path::MAIN_SEPARATOR);
    }
    (directory, name.to_string_lossy().into_owned())
}

/// The transient right-side text. Precedence is unchanged from the
/// 0.15 modeline: a live preview (or its query error) beats the free
/// pending line, which beats the walker's structural state, which
/// beats save/load activity, which beats the last message. Nothing is
/// filtered here — whatever wins is what the user sees.
fn transient(editor: &Editor) -> String {
    let preview = match editor.preview() {
        Ok(Some((_, spec))) => Some(spec),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    preview
        .or_else(|| {
            (editor.pending.is_active() && !cmd_card_active(editor))
                .then(|| editor.pending.text().trim_end_matches('\r').to_owned())
        })
        .or_else(|| {
            (!editor.walker.prefix_display().is_empty() || !editor.walker.state.empty())
                .then(|| editor.walker.display())
        })
        .or_else(|| editor.io_status().map(str::to_owned))
        .or_else(|| (!editor.message.is_empty()).then(|| editor.message.clone()))
        .unwrap_or_default()
}
