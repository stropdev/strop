//! Insert mode: text in, Esc out. Every session is recorded so
//! dot-repeat can replay both the command and the inserted text.
//! Newlines preserve the buffer's line-ending style (0031 R5): Enter
//! and replay both insert the document's own break, CRLF or LF.

use super::{Editor, Mode};
use strop_core::frontend_input::Key;
pub(crate) mod replay;

impl Editor {
    /// The buffer's line-ending for NEW lines: the first line's break
    /// decides (vim's fileformat rule on one buffer) — `\r\n` when
    /// line 0 ends CRLF, `\n` otherwise. A break-less buffer defaults
    /// to LF.
    pub(crate) fn newline_str(&self) -> &'static str {
        let buf = self.buf();
        if buf.len_bytes() == 0 {
            return "\n";
        }
        // line_end(0) is the content end: the byte at it is the break
        // (or the \r of a CRLF pair) when the file has a first break
        let e = buf.line_end(0);
        match buf.byte_at(e) {
            Some(b'\r') if buf.byte_at(e + 1) == Some(b'\n') => "\r\n",
            Some(b'\n') => "\n",
            _ => "\n",
        }
    }

    /// Enter insert mode and start recording (dot-repeat, 0001 §2.1).
    /// `keys` is what got us here (`i`, `o`, `ci[`, …).
    pub(crate) fn enter_insert_from(&mut self, keys: &str) {
        // an insert session is one undo unit (with the change op, if any, that
        // opened it); plain entries open a fresh transaction
        self.tx_begin();
        if !matches!(keys, "o" | "O") {
            self.insert_open = None;
        }
        self.mode = Mode::Insert;
        self.recording_insert = Some(String::new());
        self.insert_replay = Some(Vec::new());
        if self.last_cmd_keys.is_empty()
            || !matches!(keys.chars().next(), Some('c') if keys.len() > 1)
        {
            // plain entries remember their own key; change ops already set
            // last_cmd_keys in execute()
            if matches!(keys, "i" | "a" | "A" | "o" | "O" | "v..." | "V...") {
                self.last_cmd_keys = keys.trim_end_matches("...").into();
                self.last_change = None;
            }
        }
    }

    pub(crate) fn insert_register(&mut self, name: Option<char>) {
        let text = self.register(name).text.clone();
        self.insert_literal(&text, replay::InsertTargets::All);
    }

    pub(crate) fn feed_insert(&mut self, key: Key) {
        if self.insert_register {
            self.insert_register = false;
            return match key {
                Key::Char('"') => self.insert_register(None),
                Key::Char(name) => self.insert_register(Some(name)),
                _ => self.message = "register insertion needs a register name".into(),
            };
        }
        // 0069 D05: a live snippet tabstop session owns Tab/Esc/etc.
        // first; printable keys fall through with the placeholder
        // replaced (see snippet.rs).
        if self.snippet.is_some() {
            use super::snippet::SnippetKeyOutcome;
            match self.snippet_key(key) {
                SnippetKeyOutcome::Consumed => return,
                SnippetKeyOutcome::EndAndForward => {}
                SnippetKeyOutcome::PassThrough => {}
            }
        }
        use super::completion::KeyDisposition;
        match self.completion_key(key) {
            KeyDisposition::Requested
            | KeyDisposition::Navigated
            | KeyDisposition::Accepted
            | KeyDisposition::Dismissed => return,
            KeyDisposition::DismissAndForward | KeyDisposition::PassThrough => {}
        }
        // Structural changes are planned against one revision before any
        // cursor mutates. A deferred request retains the same ordered input.
        let trigger = match key {
            Key::Enter => Some(super::structural_indent::Trigger::Enter),
            Key::Char(character @ ('}' | ']' | ')')) if self.closer_needs_alignment() => {
                strop_syntax::DelimiterKind::closing(character)
                    .map(super::structural_indent::Trigger::Closer)
            }
            _ => None,
        };
        if let Some(trigger) = trigger {
            let Some(edits) = self.resolve_indentation(trigger) else {
                return;
            };
            let primary = edits.iter().find(|edit| edit.cursor.get() == self.head());
            let removed = primary.map_or(0, |edit| {
                let count = edit.cursor.get().saturating_sub(edit.start.get());
                self.recording_insert
                    .as_ref()
                    .filter(|recording| {
                        recording.len() >= count
                            && recording.as_bytes()[recording.len() - count..]
                                .iter()
                                .copied()
                                .eq(self
                                    .buf()
                                    .text()
                                    .byte_slice(edit.start.get()..edit.cursor.get())
                                    .bytes())
                    })
                    .map_or(0, |_| count)
            });
            if let (Some(recording), Some(edit)) = (&mut self.recording_insert, primary) {
                recording.truncate(recording.len() - removed);
                recording.push_str(&edit.text[..edit.landing]);
            }
            self.record_insert_key(key);
            self.apply_indentation(&edits);
            self.last_insert_pos = Some(self.head());
            return;
        }
        if matches!(
            key,
            Key::Char(_) | Key::Tab | Key::Backspace | Key::Left | Key::Right | Key::Up | Key::Down
        ) {
            self.record_insert_key(key);
        }
        // gi remembers the insert caret; Esc leaves the final position behind.
        self.last_insert_pos = Some(self.head());
        match key {
            Key::CtrlC => self.feed_insert(Key::Esc),
            Key::CtrlW | Key::CtrlX | Key::CtrlD | Key::CtrlO | Key::CtrlL | Key::CtrlSpace => {
                if key == Key::CtrlL {
                    self.needs_repaint = true; // desync recovery
                }
            }
            Key::Esc => {
                let count = std::mem::replace(&mut self.insert_count, 1);
                if count > 1 {
                    let keys = self.insert_replay.clone().unwrap_or_default();
                    let open = self.insert_open.is_some();
                    for _ in 1..count {
                        if open {
                            self.replay_insert_key(Key::Enter);
                        }
                        for step in &keys {
                            self.replay_insert_step(step.clone());
                        }
                    }
                    self.replay_insert_key(Key::Esc);
                    return;
                }
                self.last_insert_replay = self.insert_replay.take().unwrap_or_default();
                self.mode = Mode::Normal;
                self.apply_pending_diagnostics();
                self.set_head(self.head().saturating_sub(1));
                let extras: Vec<strop_core::selection::Selection> = self
                    .extra_selections()
                    .iter()
                    .map(|s| strop_core::selection::Selection {
                        anchor: s.anchor,
                        head: s.head.saturating_sub(1),
                    })
                    .collect();
                self.sels_mut().set_extra_selections(extras);
                if let Some(rec) = self.recording_insert.take() {
                    if self.block_insert_state.is_some() {
                        self.block_replicate(&rec);
                    }
                    self.last_insert = Some(rec);
                } else {
                    self.insert_open = None;
                }
                self.clamp_cursor();
                self.normalize_cursors();
                self.tx_commit(); // the insert session is one undo unit
            }
            Key::Backspace => {
                // cascade: every cursor deletes one char back, bottom-up
                // so positions never shift mid-batch (0013 §3)
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup();
                positions.retain(|&p| p > 0); // cursors at 0 can't delete
                let edits: Vec<_> = positions
                    .iter()
                    .map(|&pos| {
                        let start = if self.buf().byte_at(pos - 1) == Some(b'\n')
                            && pos > 1
                            && self.buf().byte_at(pos - 2) == Some(b'\r')
                        {
                            pos - 2
                        } else {
                            self.buf().clamp_boundary(pos - 1)
                        };
                        super::structural_indent::Edit {
                            cursor: strop_core::id::ByteOffset::new(pos),
                            start: strop_core::id::ByteOffset::new(start),
                            end: strop_core::id::ByteOffset::new(pos),
                            text: String::new(),
                            landing: 0,
                        }
                    })
                    .collect();
                let removed_chars = edits
                    .iter()
                    .find(|edit| edit.cursor.get() == self.head())
                    .map_or(0, |edit| {
                        self.buf()
                            .text()
                            .byte_slice(edit.start.get()..edit.end.get())
                            .len_chars()
                    });
                if !edits.is_empty() {
                    self.apply_indentation(&edits);
                    if let Some(rec) = &mut self.recording_insert {
                        for _ in 0..removed_chars {
                            rec.pop();
                        }
                    }
                }
            }
            Key::Enter => unreachable!("structural Enter handled before dispatch"),
            // arrows move in insert too (vim) — char-boundary honest
            Key::Left => {
                let start = self.buf().line_start(self.buf().line_of(self.head()));
                if self.head() > start {
                    self.set_head(self.buf().clamp_boundary(self.head() - 1));
                }
            }
            Key::Right => {
                let end = self.buf().line_end(self.buf().line_of(self.head()));
                if self.head() < end {
                    self.set_head(self.buf().ceil_boundary(self.head() + 1));
                }
            }
            Key::Up | Key::Down => {
                let line = self.buf().line_of(self.head());
                let target = if key == Key::Up {
                    line.saturating_sub(1)
                } else {
                    (line + 1).min(self.buf().len_lines().saturating_sub(1))
                };
                let col = self
                    .buf()
                    .col_of(self.head())
                    .min(self.buf().line_end(target) - self.buf().line_start(target));
                self.set_head(
                    self.buf()
                        .clamp_boundary(self.buf().line_start(target) + col),
                );
            }
            Key::CtrlR => self.insert_register = true,
            Key::CtrlU
            | Key::CtrlF
            | Key::CtrlB
            | Key::CtrlCaret
            | Key::CtrlV
            | Key::CtrlN
            | Key::CtrlP
            | Key::CtrlY
            | Key::CtrlE
            | Key::Backtab => {}
            Key::Tab => {
                // vim: Tab inserts the indent unit — a tab or the
                // document's spaces (config indent_style; detection may
                // have resolved this buffer's own).
                let unit = self.cur_indent().unit();
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup();
                let width = unit.len();
                for &pos in positions.iter().rev() {
                    self.buf_mut().insert(pos, &unit);
                }
                self.remap_after_mirrored_edit(&positions, width as isize);
                if let Some(rec) = &mut self.recording_insert {
                    rec.push_str(&unit);
                }
            }
            Key::Char(c) => {
                let mut tmp = [0u8; 4];
                let encoded = c.encode_utf8(&mut tmp);
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup(); // stacked cursors edit once
                for &pos in positions.iter().rev() {
                    self.buf_mut().insert(pos, encoded);
                }
                self.remap_after_mirrored_edit(&positions, c.len_utf8() as isize);
                if let Some(rec) = &mut self.recording_insert {
                    rec.push(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn dot_repeat_includes_insert_mode_tab() {
        let mut editor = Editor::new(Buffer::from_text("ab\ncd\n"));
        editor.config = crate::config::Config {
            indent_style: crate::config::IndentStyle::Tabs,
            ..Default::default()
        };
        editor.reresolve_indents();

        editor.feed_text("i");
        editor.feed(Key::Tab);
        editor.feed_text("Z");
        editor.feed(Key::Esc);
        assert_eq!(editor.buf().text().to_string(), "\tZab\ncd\n");

        editor.feed_text("j0.");
        assert_eq!(editor.buf().text().to_string(), "\tZab\n\tZcd\n");
        assert_eq!(editor.head(), editor.buf().line_start(1) + 1);
    }
}
