//! Resolve every cursor before publishing a motion; query failures are atomic.
use crate::editor::Editor;
use strop_grammar::{self as grammar, Command};

impl Editor {
    pub(crate) fn move_cursor(&mut self, command: &Command) {
        if self.block_vertical(command) {
            return;
        }
        self.view_mut().desired_column = None;
        let cursors = self.all_cursors();
        let resolutions = match grammar::resolve_many(self.buf(), &cursors, command) {
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
        if primary.is_some()
            && matches!(
                command.target,
                grammar::Target::Motion(
                    grammar::Motion::FirstLine
                        | grammar::Motion::LastLine
                        | grammar::Motion::MatchPair
                        | grammar::Motion::Search(_)
                        | grammar::Motion::SearchBackward(_)
                )
            )
        {
            self.push_jump();
        }
        let head = primary
            .map(|resolved| grammar::cursor_after(self.buf(), self.head(), command, resolved));
        let extras: Vec<_> = cursors
            .iter()
            .zip(&resolutions)
            .skip(1)
            .map(|(&cursor, resolved)| {
                let head = resolved.as_ref().map_or(cursor, |resolved| {
                    grammar::cursor_after(self.buf(), cursor, command, resolved)
                });
                self.clamp_pos(head)
            })
            .collect();
        self.note_search(command);
        if let Some(head) = head {
            self.set_head(head);
        }
        self.sels_mut().set_extras(extras);
        self.clamp_cursor();
        self.normalize_cursors();
        if is_search && primary.is_none() {
            self.message = "pattern not found".into();
        }
    }
}
