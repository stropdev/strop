//! Visual mode (charwise `v`, linewise `V`, block `Ctrl-V`): motions
//! extend the selection, verbs consume it. Its keys are the binding
//! table's `visual` section, walked before the grammar (0095 §2.3);
//! text prompts route through the shared pending owner — Visual has no
//! input state of its own (R7).

use strop_core::Range;
use strop_grammar::{self as grammar, Op};

use super::input::Action;
use super::{Editor, Mode};
use crate::keymap::Handler;
use strop_core::frontend_input::Key;

pub(crate) mod verbs;

/// The Visual selection last left: `gv` restores it, `'<` and `'>` are
/// its ends.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LastVisual {
    pub(crate) anchor: usize,
    pub(crate) head: usize,
    pub(crate) mode: Mode,
}

impl Editor {
    pub(crate) fn feed_visual(&mut self, key: Key) {
        // gv's memory: the live visual range, refreshed per key — any
        // exit path (Esc, operator, yank) leaves the last range behind
        let p = self.sels().primary();
        self.last_visual = Some(LastVisual {
            anchor: p.anchor,
            head: p.head,
            mode: self.mode,
        });
        if key == Key::Esc {
            self.mode = Mode::Normal;
            self.walker.clear();
            self.clamp_cursor();
            return;
        }
        let action = self.walker.feed_visual(key);
        self.dispatch_visual_action(action);
    }

    /// `v` / `V` / `Ctrl-V`: enter that kind of Visual. Inside Visual the
    /// same key leaves it and another switches kind, keeping the anchor.
    pub(crate) fn enter_visual_mode(&mut self, target: Mode) {
        let visual = matches!(
            self.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        );
        if self.mode == target {
            self.mode = Mode::Normal;
            self.clamp_cursor();
        } else if visual {
            self.mode = target;
        } else {
            // visual mode is primary-only — extras collapse (0013)
            self.sels_mut().collapse_extras();
            let head = self.head();
            self.sels_mut().stretch_primary(head, head);
            self.mode = target;
        }
    }

    /// A text object in Visual: select it, or extend the selection by it.
    pub(crate) fn select_visual_object(&mut self, command: &grammar::Command) {
        if self.defer_resolution(
            command,
            self.all_cursors(),
            super::resolution::ResolutionPurpose::VisualObject,
        ) {
            return;
        }
        let selected = grammar::select_object(self.buf(), self.anchor(), self.head(), command);
        // occurrence selections are real: every extra re-anchors on ITS
        // object (0049 §7.4 — this was primary-only)
        let extras = match self.resolved_many(command, &self.all_cursors()) {
            Ok(resolutions) => self
                .extra_selections()
                .iter()
                .zip(resolutions.iter().skip(1))
                .map(|(old, resolved)| match resolved {
                    Some(resolved) => strop_core::selection::Selection {
                        anchor: resolved.range.start.get(),
                        head: self
                            .buf()
                            .clamp_boundary(resolved.range.end.get().saturating_sub(1)),
                    },
                    None => *old,
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if let Some(selected) = selected {
            self.sels_mut()
                .stretch_primary(selected.anchor, selected.head);
            self.mode = if selected.linewise {
                Mode::VisualLine
            } else {
                Mode::Visual
            };
        }
        self.sels_mut().set_extra_selections(extras);
    }

    /// d/y/c/x over every charwise selection (0049 §7.4): occurrence
    /// selections are real ranges, so each one is yanked/deleted/
    /// changed — the batch is ONE undo unit, the register one
    /// newline-joined characterwise text (the normal cascade's rule).
    fn visual_operate_cascade(&mut self, op: Op, named: Option<char>) {
        let mut spans: Vec<((usize, usize), bool)> = std::iter::once((self.sels().primary(), true))
            .chain(self.extra_selections().iter().copied().map(|s| (s, false)))
            .map(|(selection, primary)| (self.selection_span(selection), primary))
            .collect();
        spans.sort_by_key(|(span, _)| span.0);
        spans.dedup_by_key(|(span, _)| *span); // identical ranges edit once
        let texts: Vec<String> = spans
            .iter()
            .map(|((start, end), _)| self.buf().slice_string(Range::charwise(*start, *end)))
            .collect();
        if op == Op::Yank {
            self.yank_register(named, super::Register::characterwise(texts.join("\n")));
            // helix rule: yank keeps the selections (and Visual mode)
            let flash = spans
                .iter()
                .find(|(_, primary)| *primary)
                .map_or(spans[0].0, |(span, _)| *span);
            self.flash(Range::charwise(flash.0, flash.1));
            return;
        }
        self.tx_begin();
        for ((start, end), _) in spans.iter().rev() {
            self.buf_mut().delete(Range::charwise(*start, *end));
        }
        self.delete_register(
            named,
            super::Register::characterwise(texts.join("\n")),
            false,
        );
        // landings: each range start minus what lower deletes removed
        // (deletes applied bottom-up above)
        let mut shift = 0usize;
        let mut landings: Vec<(usize, bool)> = Vec::with_capacity(spans.len());
        for ((start, end), primary) in &spans {
            landings.push((*start - shift, *primary));
            shift += end - start;
        }
        let head = landings
            .iter()
            .find(|(_, primary)| *primary)
            .map(|(start, _)| *start)
            .unwrap_or(self.head());
        self.set_head(head);
        self.sels_mut().set_extra_selections(
            landings
                .iter()
                .filter(|(_, primary)| !*primary)
                .map(|(start, _)| strop_core::selection::Selection::cursor(*start)),
        );
        self.mode = Mode::Normal;
        if op == Op::Change {
            // no commit: the insert session closes the undo unit
            self.enter_insert_from("v...");
        } else {
            self.tx_commit();
        }
        self.clamp_cursor();
        self.flash(Range::charwise(self.head(), self.head()));
    }

    fn toggle_visual_comments(&mut self) {
        let column = self.buf().col_of(self.head());
        let (first, _) = verbs::selected_lines(self);
        let lines = self.selected_line_range();
        self.mode = Mode::Normal;
        self.set_head(self.buf().line_start(first) + column);
        self.toggle_comments(lines);
    }

    /// Toggle line comments on whole lines (`gc`, Visual `Ctrl-C`); the
    /// cursor keeps its line and column.
    pub(crate) fn toggle_comments(&mut self, lines: Range) {
        let head = self.head();
        let path = self.buf().path.clone();
        let edits = match strop_syntax::comments::toggle(path.as_deref(), self.buf(), lines) {
            Ok(edits) => edits,
            Err(error) => {
                // Neovim: with no comment syntax nothing changes, and the
                // buffer is 'modified' all the same
                self.buf_mut().mark_dirty();
                self.message = error;
                return;
            }
        };
        self.tx_begin();
        if let Err(error) = self.apply(
            self.current(),
            self.buf().revision(),
            super::transact::ChangeSet {
                edits,
                undo_open: true,
            },
        ) {
            self.message = error.to_string();
            return;
        }
        self.tx_commit();
        self.set_head(head);
        self.clamp_cursor();
        self.flash(Range::charwise(lines.start, lines.start));
        self.last_insert = None;
    }

    /// One typed visual action: motions extend, objects select, the
    /// table's rows act on the selection.
    fn dispatch_visual_action(&mut self, action: Action) {
        match action {
            Action::Pending => {}
            Action::Invalid(keys) => self.message = format!("not an editor command: {keys}"),
            Action::Unbound(keys) => self.message = format!("{keys} is not bound"),
            Action::QueryError(error) => self.message = error.to_string(),
            Action::EnterText { sigil, state } => self.begin_text_line(sigil, state),
            Action::Grammar(command) if command.op.is_none() => match command.target {
                grammar::Target::Object { .. } => self.select_visual_object(&command),
                grammar::Target::Match { .. } => self.select_match(&command),
                _ => self.move_cursor(&command),
            },
            Action::Grammar(_) => self.message = "operator needs a visual selection".into(),
            Action::Row {
                row,
                count,
                register,
                arg,
                ..
            } if matches!(row.handler, Handler::Visual(_)) => {
                if let Handler::Visual(verb) = row.handler {
                    crate::editor::visual::verbs::visual_verb(
                        self,
                        verb,
                        count,
                        register,
                        arg.unwrap_or('\0'),
                    );
                }
            }
            Action::Row { row, .. } if row.id == "git-file-history" => {
                // visual Space g h: history of the selected lines (0014 §4)
                let (first, last) = crate::editor::visual::verbs::selected_lines(self);
                self.open_line_history(first + 1, last + 1);
            }
            Action::Row { row, .. } if row.id == "clip-yank" => {
                // visual Space y: yank the selection to the clipboard
                if let Some(range) = self.visual_range() {
                    let linewise = self.mode == Mode::VisualLine;
                    let text = self.buf().slice_string(range);
                    let mut register = if linewise {
                        super::Register::linewise(text)
                    } else {
                        super::Register::characterwise(text)
                    };
                    register.file_provenance =
                        self.capture_filename_register([(range, linewise)], false, false);
                    self.yank_register(Some('+'), register);
                    self.flash(range);
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
            }
            // Visual-section Leaf rows (0049 §7: gb/gB occurrence
            // adding, v/V switching) execute their handler.
            Action::Row { row, key, .. } if row.sections.contains(&"visual") => {
                if let Handler::Leaf(f) = row.handler {
                    f(self, key);
                }
            }
            Action::Row { .. } => self.message = "command unavailable in visual mode".into(),
        }
    }

    /// `S<c>`: wrap the selection in a pair.
    fn visual_surround(&mut self, c: char) {
        let Some(range) = self.visual_range() else {
            return;
        };
        let pair = match c {
            'b' | '(' | ')' => ('(', ')'),
            'B' | '{' | '}' => ('{', '}'),
            'r' | '[' | ']' => ('[', ']'),
            'a' | '<' | '>' => ('<', '>'),
            q => (q, q),
        };
        // two distinct boundary insertions into pre-edit
        // coordinates — one validated ChangeSet, no ordering
        // ambiguity (R8)
        let changes = crate::editor::transact::ChangeSet {
            edits: vec![
                strop_core::Replacement::new(
                    Range::charwise(range.start, range.start),
                    pair.0.to_string(),
                ),
                strop_core::Replacement::new(
                    Range::charwise(range.end, range.end),
                    pair.1.to_string(),
                ),
            ],
            undo_open: false,
        };
        if let Err(error) = self.apply(self.current(), self.buf().revision(), changes) {
            self.message = error.to_string();
            return;
        }
        self.mode = Mode::Normal;
        self.flash(Range::charwise(
            range.start,
            range.end.get() + pair.0.len_utf8() + pair.1.len_utf8(),
        ));
        self.last_cmd_keys = format!("vS{c}"); // replay is visual-mode replay; approximated
        self.last_insert = None;
    }

    pub fn visual_range(&self) -> Option<Range> {
        match self.mode {
            Mode::Visual => {
                // charwise-inclusive spans whole CHARS (0020 §10): the
                // +1 from a multibyte lead landed mid-char and panicked
                // (an end standing on a line's end takes its line break:
                // `v$`, or `vj` onto a shorter line)
                // — except the buffer's last line, which has none to take
                let last = self.anchor().max(self.head());
                let line = self.buf().line_of(last);
                let at_buffer_end =
                    line >= self.buf().last_content_line() && last >= self.buf().line_end(line);
                let (s, e) = (
                    self.anchor().min(self.head()),
                    if at_buffer_end {
                        last
                    } else {
                        self.buf().ceil_boundary(last + 1)
                    },
                );
                Some(Range::charwise(s, e.min(self.buf().len_bytes())).with_inclusive(true))
            }
            Mode::VisualLine => Some(self.selected_line_range()),
            _ => None,
        }
    }

    fn selected_line_range(&self) -> Range {
        let (a, b) = crate::editor::visual::verbs::selected_lines(self);
        let start = self.buf().line_start(a);
        let end = if b + 1 >= self.buf().len_lines() {
            self.buf().len_bytes()
        } else {
            self.buf().line_start(b + 1)
        };
        Range::linewise(start, end)
    }

    /// The keys that redo a Visual operator over a selection spanning
    /// lines: the same line and column deltas from the cursor.
    fn visual_replay_keys(&self, op: Op) -> String {
        let anchor = self.anchor();
        let head = self.head();
        let buffer = self.buf();
        let rope = buffer.text();
        let anchor_line = buffer.line_of(anchor);
        let head_line = buffer.line_of(head);
        let column = |position| {
            let line_start = buffer.line_start(buffer.line_of(position));
            rope.byte_to_char(position) - rope.byte_to_char(line_start)
        };
        let anchor_column = column(anchor);
        let head_column = column(head);

        let mut keys = String::from("v");
        Self::push_counted_motion(
            &mut keys,
            anchor_line.abs_diff(head_line),
            if head_line < anchor_line { 'k' } else { 'j' },
        );
        Self::push_counted_motion(
            &mut keys,
            anchor_column.abs_diff(head_column),
            if head_column < anchor_column {
                'h'
            } else {
                'l'
            },
        );
        // the Visual key of the operator: `gU`, not Normal's `U`
        keys.push_str(op.key());
        keys
    }

    fn push_counted_motion(keys: &mut String, count: usize, motion: char) {
        if count == 0 {
            return;
        }
        if count > 1 {
            keys.push_str(&count.to_string());
        }
        keys.push(motion);
    }
}
