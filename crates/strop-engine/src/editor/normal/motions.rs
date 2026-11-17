//! Resolve every cursor before publishing a motion; query failures are atomic.
use crate::editor::Editor;
use strop_grammar::{self as grammar, Command, Parse};

impl Editor {
    pub(crate) fn move_cursor(&mut self, command: &Command) {
        let bound = match crate::editor::normal::bind::bind_command(self, command) {
            Ok(bound) => bound.into_owned(),
            Err(message) => {
                self.message = message;
                return;
            }
        };
        let command = &bound;
        if self.defer_resolution(
            command,
            self.all_cursors(),
            super::super::resolution::ResolutionPurpose::Motion,
        ) {
            return;
        }
        if self.block_vertical(command) {
            return;
        }
        // Vertical motions keep the column the cursor wants, through
        // shorter lines (Vim's curswant); `$` wants the line's end.
        let wanted = match &command.target {
            grammar::Target::Motion(
                grammar::Motion::Down
                | grammar::Motion::Up
                | grammar::Motion::ScreenDown { .. }
                | grammar::Motion::ScreenUp { .. }
                | grammar::Motion::FirstLine
                | grammar::Motion::LastLine
                | grammar::Motion::Window { .. },
            ) => Some(
                self.view()
                    .desired_column
                    .unwrap_or_else(|| self.cluster_cells(self.head()).1 - 1),
            ),
            grammar::Target::Motion(grammar::Motion::LineEnd) => {
                Some(strop_core::id::DisplayColumn::new(usize::MAX / 2))
            }
            _ => None,
        };
        let cursors = self.all_cursors();
        let resolutions = match self.resolved_many(command, &cursors) {
            Ok(resolutions) => resolutions,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        let primary = resolutions.first().and_then(Option::as_ref);
        let is_search = matches!(
            command.target,
            grammar::Target::Motion(
                grammar::Motion::Search(_) | grammar::Motion::SearchBackward(_)
            )
        );
        let head = primary
            .map(|resolved| grammar::cursor_after(self.buf(), self.head(), command, resolved));
        let search_jump = is_search && primary.is_some();
        let moving_jump = head.is_some_and(|head| head != self.head())
            && matches!(
                command.target,
                grammar::Target::Motion(
                    grammar::Motion::FirstLine
                        | grammar::Motion::LastLine
                        | grammar::Motion::ParagraphBackward
                        | grammar::Motion::ParagraphForward
                        | grammar::Motion::MatchPair
                )
            );
        if search_jump || moving_jump {
            self.push_jump();
        }
        let extras: Vec<_> = {
            let olds = self.extra_selections().to_vec();
            olds.iter()
                .zip(resolutions.iter().skip(1))
                .map(|(old, resolved)| {
                    // stretched extras (occurrences, 0049 §7.4) move
                    // their head and keep their anchor and direction
                    let head = resolved.as_ref().map_or(old.head, |resolved| {
                        grammar::cursor_after(self.buf(), old.head, command, resolved)
                    });
                    strop_core::selection::Selection {
                        anchor: old.anchor,
                        head: self.clamp_pos(head),
                    }
                })
                .collect()
        };
        self.note_search(command);
        if let Some(head) = head {
            self.set_head(head);
        }
        self.sels_mut().set_extra_selections(extras);
        self.clamp_cursor();
        self.normalize_cursors();
        if let Some(column) = wanted {
            if head.is_some() {
                let line = self.buf().line_of(self.head());
                self.land_at_cell(line, column);
            }
            self.view_mut().desired_column = Some(column);
        }
        if is_search && primary.is_none() {
            self.fail("pattern not found".into());
        }
    }

    /// vim Enter: [count] lines down, first non-blank. With the blame
    /// gutter on, Enter dives into the line's commit instead (0011 §3).
    pub fn enter_pub(&mut self) {
        if self.dive_from_blame() {
            return;
        }
        // 0097 §3.2: Enter in a collection opens the file at the cursor;
        // on a row that shows no file it keeps its vim meaning.
        if self.collection_enter() {
            return;
        }
        // 0102: Enter in a folder opens the entry under the cursor.
        if self.directory_key(strop_core::frontend_input::Key::Enter) {
            return;
        }
        let n = self.walker.state.count1.unwrap_or(1);
        let line = (self.buf().line_of(self.head()) + n).min(self.buf().last_content_line());
        let s = self.buf().line_start(line);
        let e = self.buf().line_end(line);
        let mut p = s;
        while p < e
            && self
                .buf()
                .byte_at(p)
                .is_some_and(|b| b == b' ' || b == b'\t')
        {
            p += 1;
        }
        self.set_head(p.min(e));
        self.clamp_cursor();
    }

    pub fn run_motion(&mut self, keys: &str) {
        match grammar::parse(keys) {
            Parse::Complete(cmd) => self.move_cursor(&cmd),
            Parse::QueryError(error) => self.message = error.to_string(),
            Parse::Incomplete | Parse::Invalid => {}
        }
    }
}
