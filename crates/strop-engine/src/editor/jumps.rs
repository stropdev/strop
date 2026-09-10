//! vim's jumplist: `ctrl-o` back, `ctrl-i` forward (in a terminal
//! ctrl-i *is* Tab — same as vim). past/future stacks: a new jump
//! truncates the future, like vim.

use super::Editor;

impl Editor {
    /// Record the current position before a jump-causing action
    /// (gd, gg, G, %, /, n, marks, buffer switches, dives). Consecutive
    /// duplicates don't pile up.
    pub(crate) fn push_jump(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        let pos = (self.current(), self.head());
        if self.jumplist_past.last() != Some(&pos) {
            self.jumplist_past.push(pos);
        }
        self.jumplist_future.clear(); // a new jump truncates the forward path
    }
    /// `ctrl-o`: one jump back. Entries whose document was closed die
    /// here, not in `jump_to` — skipping there would leave the current
    /// position stranded on the future stack.
    pub(crate) fn jump_back(&mut self) {
        while let Some(pos) = self.jumplist_past.last().copied() {
            self.jumplist_past.pop();
            if self.docs.get(pos.0).is_none() {
                continue; // the document is closed; the entry dies quietly
            }
            self.jumplist_future.push((self.current(), self.head()));
            self.jump_to(pos);
            return;
        }
        self.message = "no jumps".into();
    }

    /// `ctrl-i` (Tab in a terminal): one jump forward. Dead entries are
    /// skipped the same way as `jump_back`.
    pub(crate) fn jump_forward(&mut self) {
        while let Some(pos) = self.jumplist_future.last().copied() {
            self.jumplist_future.pop();
            if self.docs.get(pos.0).is_none() {
                continue;
            }
            self.jumplist_past.push((self.current(), self.head()));
            self.jump_to(pos);
            return;
        }
        self.message = "at newest jump".into();
    }
    /// Land on a jumplist position: switch document when needed.
    /// Callers (`jump_back`/`jump_forward`) have already dropped entries
    /// whose document is gone.
    fn jump_to(&mut self, (buffer, offset): (strop_core::id::DocumentId, usize)) {
        debug_assert!(self.docs.get(buffer).is_some(), "jump_to: dead entry");
        if buffer != self.current() {
            self.switch_to(buffer);
            self.discover_git();
        }
        self.set_head(
            self.buf()
                .clamp_boundary(offset.min(self.buf().len_bytes())),
        );
        self.clamp_cursor();
        self.flash(strop_core::Range::charwise(self.head(), self.head()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;
    use strop_core::Buffer;

    #[test]
    fn jumplist_walks_back_and_forward_across_buffers() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("j"); // cursor on line 2
        e.push_jump();
        e.feed_text("G"); // last line
        assert_eq!(e.buf().line_of(e.head()), 2);
        e.jump_back();
        assert_eq!(e.buf().line_of(e.head()), 1, "ctrl-o back to line 2");
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.head()), 2, "ctrl-i forward again");
    }

    #[test]
    fn new_jump_truncates_the_forward_path() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\nd\n"));
        e.push_jump();
        e.feed_text("jj");
        e.push_jump();
        e.feed_text("j"); // line 4
        e.jump_back(); // line 3
        e.push_jump();
        e.feed_text("k"); // line 2 — new jump kills the future
        assert!(e.jumplist_future.is_empty());
        e.jump_forward();
        assert!(e.message.contains("newest"));
    }

    #[test]
    fn search_then_ctrl_o_ctrl_i() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo hone\nthree\n"));
        e.feed_text("/hone\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "landed on the match");
        e.feed(crate::editor::Key::CtrlO);
        assert_eq!(e.buf().line_of(e.head()), 0, "ctrl-o back to the top");
        e.feed(crate::editor::Key::Tab); // ctrl-i in a terminal
        assert_eq!(e.buf().line_of(e.head()), 1, "ctrl-i forward again");
    }

    #[test]
    fn search_lands_with_jump_recorded() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo hone\nthree\n"));
        e.feed_text("/hone\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "landed on the match");
        assert_eq!(e.jumplist_past.len(), 1, "the jump was recorded");
    }

    #[test]
    fn ctrl_o_skips_jumps_into_closed_buffers() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("j"); // line 2
        e.push_jump(); // (docA, line 2)
        e.feed_text("G"); // line 3
        let b = e.docs.insert(Document::output(Buffer::from_text("x\ny\n")));
        e.switch_to(b);
        e.push_jump(); // (docB, 0) — dead after the close below
        e.close_buffer(true); // back to docA, cursor on line 3
        e.jump_back();
        // the dead entry is skipped; the LIVE docA entry is the landing
        assert_eq!(e.buf().line_of(e.head()), 1);
        e.jump_back();
        assert_eq!(e.message, "no jumps");
    }
}
