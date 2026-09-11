//! Resolve every cursor before publishing a motion; query failures are atomic.
use crate::editor::Editor;
use strop_grammar::{self as grammar, Command};

impl Editor {
    pub(crate) fn move_cursor(&mut self, command: &Command) {
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
        self.view_mut().desired_column = None;
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
        if is_search && primary.is_none() {
            self.message = "pattern not found".into();
        }
    }
}
