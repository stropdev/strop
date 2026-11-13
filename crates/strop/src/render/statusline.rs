//! The modeline (0032, 0088 §2, 0098): one quiet row on the surface
//! plane at the terminal's bottom edge.
//!
//! `  NORMAL    main*  crates/strop/src/render/statusline.rs ●      d2        LSP!  ●1 ▲2   17%   98:9  `
//!
//! The left half says what this is: a fixed 10-cell mode chip, so the
//! path never jumps; recording; branch; file. The right half says what
//! is happening: the pending keys in their slot (the grammar's spec
//! footer, in the accent — 0001 §4), then diagnostics only when
//! non-zero, a 4-cell `LSP!` badge only on failure, `Spaces:N` only
//! when it differs from the configured default, and where the cursor
//! is, in fixed-width fields. Messages never live here: they have their
//! own echo row (`render/echo.rs`).
//!
//! Distances come from one scale (0098 §3.1): marks are glued to their
//! owner, one cell parts the members of a group, two cells part groups.
//! Every width is a display cell, never a character. Which part gives
//! way under pressure is `layout.rs`'s one table: the lowest part
//! shrinks to its floor before it is dropped — the directory first, the
//! filename last. Service failures keep their badge through every
//! degradation, including the chip-and-position-only fallback.

use std::borrow::Cow;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::Editor;

use super::card::{hint_spans, Hint};
use super::clip::{clip_end, clip_start, width};
use super::file_name;
use super::style::Tokens;

mod chip;
mod layout;
mod services;
mod spec;
#[cfg(test)]
mod tests;

use layout::{Grants, Part};

pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 || editor.panes().is_empty() {
        return; // a 0-sized resize must not underflow (0027 §2)
    }
    let cells = area.width as usize;
    let row = Modeline::gather(editor, t);
    let spans = match row.fit(cells) {
        Some(grants) => row.spans(t, &grants),
        None => {
            let (chip, chip_cells) = row.bare(cells);
            row.bare_spans(t, &chip, chip_cells, cells)
        }
    };
    let rect = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
        rect,
    );
}

/// Everything the row can show. Strings are gathered (and sanitized)
/// once; [`Modeline::fit`] then decides how many cells each part gets.
/// Width fitting borrows the gathered fields; spans are allocated only
/// for the final row.
struct Modeline {
    chip: String, // exactly `chip::WIDTH` cells
    chip_color: Color,
    /// `REC` while a terminal captures, `REC @a` while a macro records.
    rec: String,
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
    /// state), in the pending slot.
    spec: String,
    /// Terminal keycap hints, whole chords only (rule 10).
    hints: Vec<(String, &'static str)>,
    /// Terminal attention badges (`out+`, `paste+`).
    attention: Vec<&'static str>,
    services: Option<services::Badge>,
    errors: usize,
    warnings: usize,
    line: usize,
    col: usize,
    lines: usize, // the last line's number: the line field's width
    percent: Option<usize>,
    search: Option<(usize, usize)>, // current and total matches
    indent: String,                 // "Spaces:2" — only when non-default (rule 4)
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
        // A folder has no indentation; its slot carries the listing's
        // own readout (`27 items`, `3 of 9 · filter rs`).
        let indent = if let Some(summary) = editor.directory_summary() {
            summary
        } else if terminal
            || (indent.width == config.tab_size && indent.style == config.indent_style)
        {
            String::new()
        } else {
            indent.label()
        };
        let rec = match editor.recording() {
            Some(register) => format!("REC @{register}"),
            None if terminal && editor.terminal_capture_enabled() => "REC".to_owned(),
            None => String::new(),
        };
        Self {
            chip,
            chip_color,
            rec: printable(rec),
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
            line,
            col,
            lines: if terminal_input { line } else { content },
            percent: (!terminal).then_some(percent),
            search: (!terminal).then(|| editor.search_position()).flatten(),
            indent,
        }
    }

    /// The finished row: the left half, the flexible gap, the right
    /// half. Each part is cut to the cells the plan gave it.
    fn spans<'a>(&'a self, t: &Tokens, grants: &Grants) -> Vec<Span<'a>> {
        let muted = Style::default().fg(t.muted);
        let warning = Style::default().fg(t.warning);
        let mut row = Row {
            spans: vec![self.chip_span(t, self.chip.as_str())],
        };
        if grants.get(Part::Rec).is_some() {
            row.group();
            row.push(
                self.rec.as_str(),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            );
        }
        if let Some(cells) = grants.get(Part::Git) {
            let marks = usize::from(self.worktree_dirty) + usize::from(self.staged_mark);
            row.group();
            row.push(clip_end(&self.git_context, cells - marks), muted);
            if self.worktree_dirty {
                row.push("*", warning);
            }
            if self.staged_mark {
                row.push("+", warning);
            }
        }
        if let Some(cells) = grants.get(Part::Name) {
            row.group();
            if let Some(cells) = grants.get(Part::Namespace) {
                row.push(clip_end(&self.namespace, cells - 1), muted);
                row.pad(1);
            }
            // the file-name idiom (0088 §2 "File names"), row weight:
            // the modeline is one quiet row, not a heading
            if let Some(cells) = grants.get(Part::Dir) {
                row.push(clip_start(&self.dir, cells), file_name::dir_style(t));
            }
            row.push(
                clip_end(&self.name, self.name_cells(cells)),
                file_name::name_style(file_name::Role::Row, t),
            );
            if self.dirty {
                row.push(" ●", warning);
            }
            if self.readonly {
                row.push(" [RO]", warning);
            }
            if let Some(mark) = &self.multicursor {
                row.pad(1);
                row.push(
                    mark.as_str(),
                    Style::default()
                        .fg(t.secondary)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        row.pad(grants.spare());
        if let Some(cells) = grants.get(Part::Spec) {
            row.group();
            row.push(clip_end(&self.spec, cells), Style::default().fg(t.accent));
            // typed keys grow to the right inside their slot
            row.pad(grants.get(Part::SlotPad).unwrap_or(0));
        }
        if grants.get(Part::Attention).is_some() {
            for badge in &self.attention {
                row.group();
                row.push(*badge, warning);
            }
        }
        if let Some(cells) = grants.get(Part::Hints) {
            let hints: Vec<Hint<'_>> = self
                .hints
                .iter()
                .take(self.hints_shown(cells))
                .map(|(key, desc)| Hint { key, desc })
                .collect();
            let mut spans = hint_spans(&hints, usize::MAX, t);
            // drop the footer's own edge padding: groups here are parted
            // by the row's separator like every other group
            spans.pop();
            let used: usize = spans.iter().skip(1).map(|span| width(&span.content)).sum();
            row.group();
            row.spans.extend(spans.into_iter().skip(1));
            row.pad(cells - used);
        }
        if let Some(badge) = self
            .services
            .as_ref()
            .filter(|_| grants.get(Part::Services).is_some())
        {
            row.group();
            row.spans.push(badge.span(t));
        }
        if grants.get(Part::Diag).is_some() {
            row.group();
            if self.errors > 0 {
                row.push(format!("●{}", self.errors), Style::default().fg(t.error));
            }
            if self.warnings > 0 {
                if self.errors > 0 {
                    row.pad(1);
                }
                row.push(format!("▲{}", self.warnings), warning);
            }
        }
        if grants.get(Part::Indent).is_some() {
            row.group();
            row.push(self.indent.as_str(), muted);
        }
        if let Some((current, total)) = self.search.filter(|_| grants.get(Part::Search).is_some()) {
            let cells = layout::digits(total);
            row.group();
            row.push(format!("{current:>cells$}/{total}"), muted);
        }
        if let Some(percent) = self.percent.filter(|_| grants.get(Part::Percent).is_some()) {
            row.group();
            row.push(format!("{percent:>3}%"), muted);
        }
        row.group();
        row.push(
            format!(
                "{:>line$}:{:<col$}",
                self.line,
                self.col,
                line = self.line_cells(),
                col = self.col_cells()
            ),
            Style::default().fg(t.secondary),
        );
        row.spans
    }

    fn chip_span<'a>(&self, t: &Tokens, text: impl Into<Cow<'a, str>>) -> Span<'a> {
        Span::styled(
            text,
            Style::default()
                .fg(t.on_accent)
                .bg(self.chip_color)
                .add_modifier(Modifier::BOLD),
        )
    }

    fn bare_spans(
        &self,
        t: &Tokens,
        chip: &str,
        chip_cells: usize,
        cells: usize,
    ) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        if chip_cells >= 1 {
            // the leading cells carry the safety marks: cut, never elide
            let shown: String = unicode_segmentation::UnicodeSegmentation::graphemes(chip, true)
                .scan(0usize, |used, grapheme| {
                    *used += width(grapheme);
                    (*used <= chip_cells).then_some(grapheme)
                })
                .collect();
            spans.push(self.chip_span(t, shown));
        }
        spans.push(Span::styled(
            clip_end(
                &format!("{}:{}", self.line, self.col),
                cells.saturating_sub(chip_cells),
            )
            .into_owned(),
            Style::default().fg(t.secondary),
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

    fn show_diag(&self) -> bool {
        self.errors > 0 || self.warnings > 0
    }
}

/// The row under construction: spans, with the spacing scale's
/// distances as the only way to add blank cells.
struct Row<'a> {
    spans: Vec<Span<'a>>,
}

impl<'a> Row<'a> {
    fn push(&mut self, text: impl Into<Cow<'a, str>>, style: Style) {
        self.spans.push(Span::styled(text, style));
    }

    fn pad(&mut self, cells: usize) {
        if cells > 0 {
            self.spans.push(Span::raw(" ".repeat(cells)));
        }
    }

    /// The separator before a group.
    fn group(&mut self) {
        self.pad(layout::SEP);
    }
}

fn printable(text: String) -> String {
    strop_core::layout::printable_text(text).into_owned()
}

/// A historical view names its captured commit, not today's worktree branch.
fn historical_commit(editor: &Editor) -> Option<&str> {
    match editor.surface() {
        Some(crate::editor::Surface::ChangedFiles { files, .. }) => Some(files.sha()),
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
        // `crates/strop-engine/` splits as `crates/` + `strop-engine/`.
        let label = editor.folder_label(&directory.location);
        let body = &label[..label.len() - 1];
        let split = body.rfind('/').map_or(0, |slash| slash + 1);
        return (label[..split].to_string(), label[split..].to_string());
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
