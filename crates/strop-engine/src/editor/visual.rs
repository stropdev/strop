//! Visual mode (charwise `v` and linewise `V`): motions extend the
//! selection, operators consume it. Structural composition shares the
//! Walker; text prompts (`space |`) route through the shared pending
//! owner — visual has no input state of its own (R7).

use strop_core::Range;
use strop_grammar::{self as grammar, Op};

use super::input::Action;
use super::{Editor, Key, Mode};

impl Editor {
    pub(crate) fn feed_visual(&mut self, key: Key) {
        // gv's memory: the live visual range, refreshed per key — any
        // exit path (Esc, operator, yank) leaves the last range behind
        let p = self.sels().primary();
        self.last_visual = Some((p.anchor, p.head));
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.walker.clear();
            }
            Key::Up if self.walker.is_ground() => self.run_motion("k"),
            Key::Down if self.walker.is_ground() => self.run_motion("j"),
            Key::Left if self.walker.is_ground() => self.run_motion("h"),
            Key::Right if self.walker.is_ground() => self.run_motion("l"),
            Key::Char('>') | Key::Char('<') if self.walker.is_ground() => {
                // visual indent: apply to every selected line, one undo
                // unit, back to normal (vim re-selects with gv)
                let Some(range) = self.visual_range() else {
                    return;
                };
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    self.mode = Mode::Normal;
                    return;
                }
                let right = key == Key::Char('>');
                self.tx_begin();
                self.apply_indent(range, right);
                self.tx_commit();
                self.mode = Mode::Normal;
                self.set_head(range.start.get());
                self.clamp_cursor();
                self.flash(Range::charwise(self.head(), self.head()));
                self.last_cmd_keys = if right { "V>" } else { "V<" }.into();
                self.last_insert = None;
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') | Key::Char('x')
                if self.walker.is_ground() && self.mode == Mode::VisualBlock =>
            {
                match key {
                    Key::Char('y') => self.block_yank(),
                    Key::Char('c') => self.block_change(),
                    _ => self.block_delete(),
                }
            }
            Key::Char('I') if self.mode == Mode::VisualBlock && self.walker.is_ground() => {
                self.block_insert(false)
            }
            Key::Char('A') if self.mode == Mode::VisualBlock && self.walker.is_ground() => {
                self.block_insert(true)
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') | Key::Char('x')
                if self.walker.is_ground() =>
            {
                let op = match key {
                    Key::Char('d') | Key::Char('x') => Op::Delete,
                    Key::Char('y') => Op::Yank,
                    _ => Op::Change,
                };
                // occurrence selections (0049 §7.4): real stretched
                // ranges cascade the operator over every occurrence
                if self.mode == Mode::Visual && self.sels().count() > 1 {
                    self.visual_operate_cascade(op);
                    return;
                }
                if self.buf().readonly && op != Op::Yank {
                    self.message = "readonly buffer".into();
                    self.mode = Mode::Normal;
                    return;
                }
                let Some(range) = self.visual_range() else {
                    return;
                };
                let linewise = self.mode == Mode::VisualLine;
                if op == Op::Change && linewise {
                    self.mode = Mode::Normal;
                    self.change_lines(None, "V...", &[(self.head(), range, true)]);
                    return;
                }
                if op == Op::Yank {
                    let text = self.buf().slice_string(range);
                    let mut register = if linewise {
                        super::Register::linewise(text)
                    } else {
                        super::Register::characterwise(text)
                    };
                    register.file_provenance =
                        self.capture_filename_register([(range, linewise)], false, false);
                    self.set_register(None, register);
                    self.flash(range);
                } else {
                    let text = self.buf().slice_string(range);
                    let provenance =
                        self.capture_filename_register([(range, linewise)], true, false);
                    let changes = crate::editor::transact::ChangeSet {
                        edits: vec![strop_core::Replacement::new(range, String::new())],
                        undo_open: false,
                    };
                    self.filename_delete_hint(range, linewise);
                    if let Err(error) = self.apply(self.current(), self.buf().revision(), changes) {
                        self.clear_filename_hint();
                        self.message = error.to_string();
                        return;
                    }
                    self.clear_filename_hint();
                    let mut register = if linewise {
                        super::Register::linewise(text)
                    } else {
                        super::Register::characterwise(text)
                    };
                    register.file_provenance = provenance;
                    self.set_register(None, register);
                    self.set_head(range.start.get());
                    self.flash(Range::charwise(self.head(), self.head()));
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
                if op == Op::Change {
                    self.enter_insert_from(if linewise { "V..." } else { "v..." });
                }
            }
            _ => {
                let action = self.walker.feed_visual(key);
                self.dispatch_visual_action(action);
            }
        }
    }

    pub(crate) fn select_visual_object(&mut self, command: &grammar::Command) {
        if self.defer_resolution(
            command,
            self.all_cursors(),
            super::resolution::ResolutionPurpose::VisualObject,
        ) {
            return;
        }
        match self.resolved_many(command, &self.all_cursors()) {
            Ok(resolutions) => {
                if let Some(Some(resolved)) = resolutions.first() {
                    let head = self.head();
                    self.sels_mut()
                        .stretch_primary(resolved.range.start.get(), head);
                    self.set_head(
                        self.buf()
                            .clamp_boundary(resolved.range.end.get().saturating_sub(1)),
                    );
                }
                // occurrence selections are real: every extra re-anchors
                // on ITS object (0049 §7.4 — this was primary-only)
                let olds = self.extra_selections().to_vec();
                let extras: Vec<strop_core::selection::Selection> = olds
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
                    .collect();
                self.sels_mut().set_extra_selections(extras);
            }
            Err(error) => self.message = error,
        }
    }

    /// d/y/c/x over every charwise selection (0049 §7.4): occurrence
    /// selections are real ranges, so each one is yanked/deleted/
    /// changed — the batch is ONE undo unit, the register one
    /// newline-joined characterwise text (the normal cascade's rule).
    fn visual_operate_cascade(&mut self, op: Op) {
        if self.buf().readonly && op != Op::Yank {
            self.message = "readonly buffer".into();
            self.mode = Mode::Normal;
            return;
        }
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
            self.set_register(None, super::Register::characterwise(texts.join("\n")));
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
        self.set_register(None, super::Register::characterwise(texts.join("\n")));
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

    /// One typed visual action: motions extend, objects select, the
    /// table's leader rows (Space y, Space g h) act on the selection,
    /// `S<c>` wraps it.
    fn dispatch_visual_action(&mut self, action: Action) {
        match action {
            Action::Pending => {}
            Action::Invalid(keys) => self.message = format!("not an editor command: {keys}"),
            Action::QueryError(error) => self.message = error.to_string(),
            Action::EnterText { sigil, state } => self.begin_text_line(sigil, state),
            Action::Grammar(command) if command.op.is_none() => {
                if let grammar::Target::Object { .. } = command.target {
                    self.select_visual_object(&command);
                } else {
                    self.move_cursor(&command);
                }
            }
            Action::Grammar(_) => self.message = "operator needs a visual selection".into(),
            Action::Row { row, .. } if row.id == "git-file-history" => {
                // visual Space g h: history of the selected lines (0014 §4)
                let a = self.buf().line_of(self.anchor()) + 1;
                let b = self.buf().line_of(self.head()) + 1;
                self.open_line_history(a.min(b), a.max(b));
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
                    self.set_register(Some('+'), register);
                    self.flash(range);
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
            }
            // Visual-mode Leaf rows (0049 §7: gb/gB occurrence adding)
            // execute their handler; the leader/git ids above keep
            // their bespoke arms.
            Action::Row { row, key, .. } if row.sections.contains(&"visual") => {
                if let crate::keymap::Handler::Leaf(f) = row.handler {
                    f(self, key);
                }
            }
            Action::Row { .. } => self.message = "command unavailable in visual mode".into(),
            Action::VisualSurround(c) => {
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    return;
                }
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
        }
    }

    pub fn visual_range(&self) -> Option<Range> {
        match self.mode {
            Mode::Visual => {
                // charwise-inclusive spans whole CHARS (0020 §10): the
                // +1 from a multibyte lead landed mid-char and panicked
                let (s, e) = (
                    self.anchor().min(self.head()),
                    self.buf().ceil_boundary(self.anchor().max(self.head()) + 1),
                );
                Some(Range::charwise(s, e.min(self.buf().len_bytes())))
            }
            Mode::VisualLine => {
                let (a, b) = (
                    self.buf().line_of(self.anchor()),
                    self.buf().line_of(self.head()),
                );
                let (a, b) = (a.min(b), a.max(b));
                let start = self.buf().line_start(a);
                let end = if b + 1 >= self.buf().len_lines() {
                    self.buf().len_bytes()
                } else {
                    self.buf().line_start(b + 1)
                };
                Some(Range::linewise(start, end))
            }
            _ => None,
        }
    }
}
