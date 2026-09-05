//! normal/motions.rs — move_cursor: resolve a motion and land.

use strop_grammar::{self as grammar, Command};

use crate::editor::Editor;

impl Editor {
    pub(crate) fn move_cursor(&mut self, cmd: &Command) {
        self.note_search(cmd);
        // jump-class motions record jumplist entries (vim: gg G % / ?)
        if matches!(
            cmd.target,
            strop_grammar::Target::Motion(
                strop_grammar::Motion::FirstLine
                    | strop_grammar::Motion::LastLine
                    | strop_grammar::Motion::MatchPair
                    | strop_grammar::Motion::Search(_)
                    | strop_grammar::Motion::SearchBackward(_)
            )
        ) {
            self.push_jump();
        }
        // the cascade (0013 §3): one scalar resolver, mapped over every
        // cursor — secondary cursors run the exact same motion
        let primary_hit = grammar::resolve(self.buf(), self.head(), cmd);
        if let Some(r) = &primary_hit {
            let land = grammar::cursor_after(self.buf(), self.head(), cmd, r);
            self.set_head(land);
        }
        // take/compute/replant: the resolver borrows self immutably
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| {
                let c = match grammar::resolve(self.buf(), s.head, cmd) {
                    Some(r) => grammar::cursor_after(self.buf(), s.head, cmd, &r),
                    None => s.head,
                };
                self.clamp_pos(c)
            })
            .collect();
        self.sels_mut().set_extras(extras);
        self.clamp_cursor();
        self.normalize_cursors();
        // vim says so when a search finds nothing
        if matches!(
            cmd.target,
            strop_grammar::Target::Motion(
                strop_grammar::Motion::Search(_) | strop_grammar::Motion::SearchBackward(_)
            )
        ) && primary_hit.is_none()
        {
            self.message = "pattern not found".into();
        }
    }
}
