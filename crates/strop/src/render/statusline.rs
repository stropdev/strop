//! The modeline (0032, 0088 §2): one quiet row on the surface plane at
//! the terminal's bottom edge.
//!
//! `[ NORMAL ]  main*  crates/strop/src/render/ statusline.rs ●        ●1 ▲2  LSP!  98:9  17%`
//!
//! A fixed 8-cell mode chip, so the path never jumps; branch and path on
//! the left; diagnostics only when non-zero, a 4-cell `LSP!` badge only
//! on failure, `Spaces:N` only when it differs from the configured
//! default, then the position — separated by two spaces. While an
//! operator pends, the grammar's spec footer replaces the right side in
//! the accent (0001 §4). Messages never
//! live here: they have their own echo row (`render/echo.rs`).
//!
//! Every width is a display cell, never a character: under pressure the
//! row yields the directory first, then the Git context, then the
//! optional signals, and only then clips the filename. Service failures
//! keep their badge through every degradation, including the chip-and-
//! position-only fallback.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::Editor;

use super::card::{hint_spans, Hint};
use super::clip::{clip_end, width};
use super::file_name;
use super::style::Tokens;

mod chip;
mod layout;
mod services;
mod spec;
#[cfg(test)]
mod tests;

/// Groups are separated by two cells.
const SEP: &str = "  ";
const SEP_W: usize = 2;
/// The halves never jam together while a spare cell exists.
const GAP: usize = 1;

pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 || editor.panes().is_empty() {
        return; // a 0-sized resize must not underflow (0027 §2)
    }
    let cells = area.width as usize;
    let mut row = Modeline::gather(editor, t);
    row.fit(cells);
    let rect = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Line::from(row.spans(t, cells))).style(Style::default().bg(t.surface)),
        rect,
    );
}

/// Everything the row can show. Strings are gathered (and sanitized)
/// once; [`Modeline::fit`] then narrows the model in place — dropping
/// or clipping fields — until it fits the terminal's cell width.
/// Width fitting borrows the gathered fields; spans are allocated only
/// for the final row, not for every degradation step.
struct Modeline {
    chip: String, // exactly `chip::WIDTH` cells unless the row is bare
    chip_color: Color,
    recording: bool,             // terminal capture: a `REC` badge after the chip
    git_context: String,         // branch or historical commit identity
    worktree_dirty: bool,        // "*" — unstaged or untracked changes
    staged_mark: bool,           // "+" — staged changes
    namespace: String,           // source identity, not optional Git context
    dir: String,                 // secondary context, trailing separator kept
    name: String,                // the part that must survive truncation
    dirty: bool,                 // " ●" after the name
    readonly: bool,              // " [RO]"
    multicursor: Option<String>, // "3×", without its leading pad
    /// The grammar footer (pending operator, live preview spec, walker
    /// state): replaces the right-side signals while present.
    spec: String,
    /// Terminal keycap hints, whole chords only (rule 10).
    hints: Vec<(String, &'static str)>,
    /// Terminal attention badges (`out+`, `paste+`).
    attention: Vec<&'static str>,
    services: Option<services::Badge>,
    errors: usize,
    warnings: usize,
    show_diag: bool,
    line: usize,
    col: usize,
    percent: Option<usize>,
    search: Option<(usize, usize)>, // current and total matches
    indent: String,                 // "Spaces:2" — only when non-default (rule 4)
    bare: bool,                     // degenerate layout: chip · position only
    bare_chip: usize,               // cells the chip may occupy in that layout
}

impl Modeline {
    fn gather(editor: &Editor, t: &Tokens) -> Self {
        let buf = editor.buf();
        let (dir, name) = file_display(editor);
        let terminal_input = editor.terminal_input_active();
        let terminal = editor.terminal_document(editor.current()).is_some();
        let (line, col) = if terminal_input {
            editor
                .terminal_frame(editor.current())
                .map_or((1, 1), |frame| {
                    (
                        usize::from(frame.cursor.row) + 1,
                        usize::from(frame.cursor.column) + 1,
                    )
                })
        } else {
            (
                buf.line_of(editor.head()) + 1,
                buf.col_of(editor.head()) + 1,
            )
        };
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
        // A terminal's `term://` name already carries its directory or
        // endpoint: no separate namespace chip.
        let namespace = if terminal {
            String::new()
        } else {
            editor
                .remote_endpoint()
                .map(|endpoint| format!("ssh:{}", endpoint.host()))
                .unwrap_or_default()
        };
        let git_context = match commit {
            Some(sha) => format!("@{}", sha.get(..8).unwrap_or(sha)),
            None if namespace.is_empty() => editor
                .git()
                .and_then(|git| git.head_branch.clone())
                .unwrap_or_default(),
            None => String::new(),
        };
        let (chip, chip_color) = chip::chip(editor, t);
        let (hints, attention) = if terminal {
            spec::terminal_hints(editor)
        } else {
            (Vec::new(), Vec::new())
        };
        let indent = editor.indentation_at(editor.current(), editor.head());
        let config = editor.config();
        let indent = if terminal
            || (indent.width == config.tab_size && indent.style == config.indent_style)
        {
            String::new()
        } else {
            indent.label()
        };
        Self {
            chip,
            chip_color,
            recording: terminal && editor.terminal_capture_enabled(),
            git_context: printable(git_context),
            namespace: printable(namespace),
            worktree_dirty: !terminal
                && commit.is_none()
                && (!editor.hunks().is_empty() || editor.hunks_untracked()),
            staged_mark: !terminal && commit.is_none() && !editor.staged_hunks().is_empty(),
            dir: printable(dir),
            name: printable(name),
            dirty: buf.is_dirty() || editor.collection_unsaved(editor.current()),
            readonly: buf.readonly && !terminal_input && !terminal,
            multicursor: (cursors > 1).then(|| format!("{cursors}×")),
            spec: printable(spec::spec(editor)),
            hints,
            attention,
            services: services::Badge::gather(editor),
            errors,
            warnings,
            show_diag: errors > 0 || warnings > 0,
            line,
            col,
            percent: (!terminal).then_some(percent),
            search: (!terminal).then(|| editor.search_position()).flatten(),
            indent,
            bare: false,
            bare_chip: 0,
        }
    }

    fn file_spans<'a>(&'a self, t: &Tokens) -> Vec<Span<'a>> {
        let mut spans = Vec::new();
        if !self.namespace.is_empty() {
            spans.push(Span::styled(
                self.namespace.as_str(),
                Style::default().fg(t.muted),
            ));
            spans.push(Span::raw(" "));
        }
        // the file-name idiom (0088 §2 "File names"), row weight: the
        // modeline is one quiet row, not a heading
        if !self.dir.is_empty() {
            spans.push(Span::styled(self.dir.as_str(), file_name::dir_style(t)));
        }
        if !self.name.is_empty() {
            spans.push(Span::styled(
                self.name.as_str(),
                file_name::name_style(file_name::Role::Row, t),
            ));
        }
        if self.dirty {
            spans.push(Span::styled(" ●", Style::default().fg(t.warning)));
        }
        if self.readonly {
            spans.push(Span::styled(" [RO]", Style::default().fg(t.warning)));
        }
        if let Some(mark) = &self.multicursor {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                mark.as_str(),
                Style::default()
                    .fg(t.secondary)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans
    }

    fn chip_span<'a>(&'a self, t: &Tokens) -> Span<'a> {
        Span::styled(
            self.chip.as_str(),
            Style::default()
                .fg(t.on_accent)
                .bg(self.chip_color)
                .add_modifier(Modifier::BOLD),
        )
    }

    /// Build the visible halves once, after all width decisions.
    fn halves<'a>(&'a self, t: &Tokens) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
        let mut left = vec![self.chip_span(t)];
        if self.recording {
            left.push(Span::raw(" "));
            left.push(Span::styled(
                "REC",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ));
        }
        if !self.git_context.is_empty() {
            left.push(Span::raw(SEP));
            left.push(Span::styled(
                self.git_context.as_str(),
                Style::default().fg(t.muted),
            ));
            if self.worktree_dirty {
                left.push(Span::styled("*", Style::default().fg(t.warning)));
            }
            if self.staged_mark {
                left.push(Span::styled("+", Style::default().fg(t.warning)));
            }
        }
        if self.has_file() {
            left.push(Span::raw(SEP));
            left.extend(self.file_spans(t));
        }
        let mut right = Vec::new();
        let group = |right: &mut Vec<Span<'a>>| {
            if !right.is_empty() {
                right.push(Span::raw(SEP));
            }
        };
        if !self.spec.is_empty() {
            right.push(Span::styled(
                self.spec.as_str(),
                Style::default().fg(t.accent),
            ));
        }
        for badge in &self.attention {
            group(&mut right);
            right.push(Span::styled(*badge, Style::default().fg(t.warning)));
        }
        if !self.hints.is_empty() {
            let hints: Vec<Hint<'_>> = self
                .hints
                .iter()
                .map(|(key, desc)| Hint { key, desc })
                .collect();
            right.extend(hint_spans(&hints, usize::MAX, t));
        }
        if let Some(badge) = &self.services {
            group(&mut right);
            right.push(badge.span(t));
        }
        if self.show_diag {
            group(&mut right);
            if self.errors > 0 {
                right.push(Span::styled(
                    format!("●{}", self.errors),
                    Style::default().fg(t.error),
                ));
            }
            if self.warnings > 0 {
                if self.errors > 0 {
                    right.push(Span::raw(" "));
                }
                right.push(Span::styled(
                    format!("▲{}", self.warnings),
                    Style::default().fg(t.warning),
                ));
            }
        }
        if !self.indent.is_empty() {
            group(&mut right);
            right.push(Span::styled(
                self.indent.as_str(),
                Style::default().fg(t.muted),
            ));
        }
        group(&mut right);
        right.push(Span::styled(self.position(), Style::default().fg(t.muted)));
        (left, right)
    }

    /// The finished row: halves joined by a flexible gap that never
    /// closes while both halves are on screen.
    fn spans<'a>(&'a self, t: &Tokens, cells: usize) -> Vec<Span<'a>> {
        if self.bare {
            return self.bare_spans(t, cells);
        }
        let (left, right) = self.halves(t);
        // the fit invariant already reserved the gap: l + GAP + r
        // fits, so this pad is at least GAP cells wide
        let pad = cells.saturating_sub(span_width(&left) + span_width(&right));
        let mut spans = left;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right);
        spans
    }

    fn bare_spans(&self, t: &Tokens, cells: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        if self.bare_chip >= 1 {
            // the leading cells carry the safety marks: cut, never elide
            let shown: String =
                unicode_segmentation::UnicodeSegmentation::graphemes(self.chip.as_str(), true)
                    .scan(0usize, |used, grapheme| {
                        *used += width(grapheme);
                        (*used <= self.bare_chip).then_some(grapheme)
                    })
                    .collect();
            spans.push(Span::styled(
                shown,
                Style::default()
                    .fg(t.on_accent)
                    .bg(self.chip_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            clip_end(&self.position(), cells.saturating_sub(self.bare_chip)).into_owned(),
            Style::default().fg(t.muted),
        ));
        spans
    }

    fn has_file(&self) -> bool {
        !self.dir.is_empty()
            || !self.name.is_empty()
            || self.dirty
            || self.readonly
            || self.multicursor.is_some()
    }
}

fn span_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| width(&span.content)).sum()
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
/// emphasized over its directory. Paths are relativized against their selected
/// project root; anything outside keeps its absolute form. Virtual buffers keep
/// their display name; a pathless, nameless buffer is the scratch.
fn file_display(editor: &Editor) -> (String, String) {
    if editor.terminal_document(editor.current()).is_some() {
        // `term://{cwd}//{N}:{program}` — the buffer's own name is the
        // whole identity; the launch directory is inside it.
        return (
            String::new(),
            editor.doc(editor.current()).label(editor.cwd()),
        );
    }
    if let Some(directory) = editor.directory() {
        let path = &directory.location.path;
        let parent = path
            .parent()
            .map(strop_workspace::directory::display_path)
            .unwrap_or_default();
        let mut name = strop_workspace::directory::display_path(
            path.file_name().map(std::path::Path::new).unwrap_or(path),
        );
        if !name.ends_with('/') {
            name.push('/');
        }
        return (
            if parent.is_empty() {
                parent
            } else {
                format!("{parent}/")
            },
            name,
        );
    }
    if let Some(file) = editor.remote_file() {
        let directory = file.path().parent().map_or_else(String::new, |parent| {
            let mut directory = parent.display().to_string();
            if !directory.ends_with('/') {
                directory.push('/');
            }
            directory
        });
        let name = file
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
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
    let root = editor
        .workspace_root_path(&strop_workspace::Filesystem::Local, path)
        .unwrap_or(editor.cwd());
    let relative = path
        .strip_prefix(root)
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
