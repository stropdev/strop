//! The modeline (0032): one quiet row at the terminal's bottom edge.
//!
//! The hierarchy borrows from the observed OMP footer — a compact mode
//! accent, a clear filename over muted directory context, and quiet,
//! separated Git/diagnostic/position groups — in strop's own palette
//! and without font-dependent glyphs. Every width here is a display
//! cell (`text`), never a character: under pressure the row yields the
//! directory first, then the Git context, then the optional signals,
//! and only then clips the filename and the transient status. Owned LSP
//! failures and presented warning/failure feedback retain compact textual
//! badges through every degradation, including the mode-and-position-only
//! fallback. Every gathered string passes the shared clipping helpers, so a
//! control byte can never reach the terminal as protocol.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use strop_engine::editor::feedback::FeedbackSeverity;
use strop_lsp::Severity;

use crate::editor::Editor;

use super::clip::{clip_end, width};
use super::{base, mode_color, muted, secondary, severity_color, text};

mod layout;
mod services;
#[cfg(test)]
mod tests;
mod transient;

/// A separator costs three cells (" │ ", padding included).
const SEP: &str = " │ ";
const SEP_W: usize = 3;
/// The halves never jam together while a spare cell exists.
const GAP: usize = 1;

pub(super) fn render(editor: &Editor, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 || editor.panes().is_empty() {
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
        Paragraph::new(Line::from(row.spans(cells))).style(Style::default().bg(
            crate::render::seed(strop_core::theme::current().statusline_bg),
        )),
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
    namespace: String,           // source identity, not optional Git context
    dir: String,                 // muted context, trailing separator kept
    name: String,                // the part that must survive truncation
    dirty: bool,                 // " ●" after the name
    readonly: bool,              // " [RO]"
    multicursor: Option<String>, // "3×", without its leading pad
    status: String,              // the transient right-side text
    feedback: Option<FeedbackSeverity>,
    compact_feedback: bool, // "!" replaces status prose under pressure
    services: services::Services,
    errors: usize,
    warnings: usize,
    show_diag: bool,
    line: usize,
    col: usize,
    percent: Option<usize>,
    search: Option<(usize, usize)>, // current and total matches
    indent: String,                 // "Spaces:4" — the effective setting (0051 R08)
    terminal: bool,
    terminal_attention: bool,
    terminal_prefix: bool,
    bare: bool,       // degenerate layout: mode accent · position only
    bare_chip: usize, // cells the chip may occupy in that layout
}

impl Modeline {
    fn gather(editor: &Editor) -> Self {
        let buf = editor.buf();
        let (dir, name) = file_display(editor);
        let terminal_input = editor.terminal_input_active();
        let terminal = editor.terminal_document(editor.current()).is_some();
        let terminal_attention = terminal
            && (editor.terminal_has_new_output(editor.current())
                || editor.terminal_paste_held(editor.current()));
        let (line, col) = if terminal_input {
            editor
                .terminal_frame(editor.current(), true)
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
        let namespace = if let Some(terminal) = editor.terminal_document(editor.current()) {
            terminal.launch.filesystem.label()
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
        Self {
            chip: if let Some(glue) = editor.picker() {
                format!(
                    " {} {} ",
                    glue.picker.kind.title().trim().to_uppercase(),
                    if glue.picker.input_normal() {
                        "NORMAL"
                    } else {
                        "INSERT"
                    }
                )
            } else if let Some(sigil) = editor.pending_sigil() {
                format!(" {sigil} INPUT ")
            } else if terminal_input {
                if editor.terminal_capture_enabled() {
                    " TERMINAL REC ".into()
                } else {
                    " TERMINAL ".into()
                }
            } else if terminal {
                if editor.terminal_capture_enabled() {
                    " SNAPSHOT REC ".into()
                } else {
                    " SNAPSHOT ".into()
                }
            } else {
                format!(
                    " {}{} ",
                    editor.mode().chip(),
                    if terminal && editor.terminal_capture_enabled() {
                        " REC"
                    } else {
                        ""
                    }
                )
            },
            accent: if terminal_input {
                super::terminal_chip()
            } else {
                mode_color(editor.mode())
            },
            git_context: printable(git_context),
            namespace: printable(namespace),
            worktree_dirty: !terminal
                && commit.is_none()
                && (!editor.hunks().is_empty() || editor.hunks_untracked()),
            staged_mark: !terminal && commit.is_none() && !editor.staged_hunks().is_empty(),
            dir: printable(dir),
            name: printable(name),
            dirty: buf.dirty || editor.collection_unsaved(editor.current()),
            readonly: buf.readonly && !terminal_input && !terminal,
            multicursor: (cursors > 1).then(|| format!("{cursors}×")),
            status: printable(transient::transient(editor)),
            feedback: editor.presented_feedback().map(|record| record.severity()),
            compact_feedback: false,
            services: services::Services::gather(editor),
            errors,
            warnings,
            show_diag: errors > 0 || warnings > 0,
            line,
            col,
            percent: (!terminal).then_some(percent),
            search: (!terminal).then(|| editor.search_position()).flatten(),
            indent: if terminal {
                String::new()
            } else {
                editor.indent_label()
            },
            terminal,
            terminal_attention,
            terminal_prefix: editor.terminal_prefix_hint().is_some(),
            bare: false,
            bare_chip: 0,
        }
    }

    fn file_spans(&self) -> Vec<Span<'_>> {
        let mut spans = Vec::new();
        if !self.namespace.is_empty() {
            spans.push(Span::styled(
                self.namespace.as_str(),
                Style::default().fg(muted()),
            ));
            spans.push(Span::raw(" "));
        }
        if !self.dir.is_empty() {
            spans.push(Span::styled(
                self.dir.as_str(),
                Style::default().fg(muted()),
            ));
        }
        if !self.name.is_empty() {
            spans.push(Span::styled(
                self.name.as_str(),
                Style::default().fg(text()),
            ));
        }
        if self.dirty {
            spans.push(Span::styled(
                " ●",
                Style::default().fg(crate::render::seed(strop_core::theme::current().warning)),
            ));
        }
        if self.readonly {
            spans.push(Span::styled(
                " [RO]",
                Style::default().fg(crate::render::seed(strop_core::theme::current().warning)),
            ));
        }
        if let Some(mark) = &self.multicursor {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                mark.as_str(),
                Style::default()
                    .fg(secondary())
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans
    }

    /// Build the visible halves once, after all width decisions.
    fn halves(&self) -> (Vec<Span<'_>>, Vec<Span<'_>>) {
        let quiet = Style::default().fg(crate::render::seed(strop_core::theme::current().border));
        let mut left = vec![
            Span::styled("▌", Style::default().fg(self.accent)),
            Span::styled(
                self.chip.as_str(),
                Style::default()
                    .fg(base())
                    .bg(self.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !self.git_context.is_empty() {
            left.push(Span::styled(SEP, quiet));
            left.push(Span::styled(
                self.git_context.as_str(),
                Style::default().fg(muted()),
            ));
            if self.worktree_dirty {
                left.push(Span::styled(
                    "*",
                    Style::default().fg(crate::render::seed(strop_core::theme::current().warning)),
                ));
            }
            if self.staged_mark {
                left.push(Span::styled(
                    "+",
                    Style::default().fg(crate::render::seed(strop_core::theme::current().warning)),
                ));
            }
        }
        if !self.dir.is_empty()
            || !self.name.is_empty()
            || self.dirty
            || self.readonly
            || self.multicursor.is_some()
        {
            left.push(Span::styled(SEP, quiet));
            left.extend(self.file_spans());
        }
        let mut right = Vec::new();
        if self.compact_feedback {
            right.push(Span::styled(
                "!",
                Style::default().fg(self.feedback_color()),
            ));
        } else if !self.status.is_empty() {
            right.push(Span::styled(
                self.status.as_str(),
                Style::default().fg(secondary()),
            ));
        }
        if !self.services.is_empty() {
            if !right.is_empty() {
                right.push(Span::styled(SEP, quiet));
            }
            self.services.extend_spans(&mut right);
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
        if !self.indent.is_empty() {
            if !right.is_empty() {
                right.push(Span::styled(SEP, quiet));
            }
            right.push(Span::styled(
                self.indent.as_str(),
                Style::default().fg(muted()),
            ));
        }
        if !right.is_empty() {
            right.push(Span::styled(SEP, quiet));
        }
        right.push(Span::styled(self.position(), Style::default().fg(muted())));
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
        let mut chip_budget = self.bare_chip;
        if self.compact_feedback && chip_budget >= 1 {
            spans.push(Span::styled(
                "!",
                Style::default().fg(self.feedback_color()),
            ));
            chip_budget -= 1;
        }
        if chip_budget >= 1 {
            spans.push(Span::styled("▌", Style::default().fg(self.accent)));
        }
        if chip_budget >= 2 {
            spans.push(Span::styled(
                clip_end(&self.chip, chip_budget - 1).into_owned(),
                Style::default()
                    .fg(base())
                    .bg(self.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            clip_end(&self.position(), cells.saturating_sub(self.bare_chip)).into_owned(),
            Style::default().fg(muted()),
        ));
        spans
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
    if let Some(terminal) = editor.terminal_document(editor.current()) {
        let directory = editor
            .terminal_launch_location(editor.current())
            .map_or_else(String::new, |location| {
                let mut label = if location.filesystem == strop_workspace::Filesystem::Local {
                    location.label()
                } else {
                    strop_workspace::directory::display_path(&location.path)
                };
                if !label.ends_with('/') {
                    label.push('/');
                }
                format!("launch {label}")
            });
        return (directory, format!("terminal #{}", terminal.session.get()));
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
