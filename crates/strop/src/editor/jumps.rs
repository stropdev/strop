//! vim's jumplist: `ctrl-o` back, `ctrl-i` forward (in a terminal
//! ctrl-i *is* Tab — same as vim). past/future stacks: a new jump
//! truncates the future, like vim.

use super::Editor;

impl Editor {
    /// Record the current position before a jump-causing action
    /// (gd, gg, G, %, /, n, marks, buffer switches, dives). Consecutive
    /// duplicates don't pile up.
    pub(crate) fn push_jump(&mut self) {
        if self.buffers.is_empty() {
            return;
        }
        let pos = (self.current, self.cursor);
        if self.jumplist_past.last() != Some(&pos) {
            self.jumplist_past.push(pos);
        }
        self.jumplist_future.clear(); // a new jump truncates the forward path
    }

    /// `ctrl-o`: one jump back.
    pub(crate) fn jump_back(&mut self) {
        if self.jumplist_past.is_empty() {
            self.message = "no jumps".into();
            return;
        }
        self.jumplist_future.push((self.current, self.cursor));
        let pos = self.jumplist_past.pop().unwrap_or((0, 0));
        self.jump_to(pos);
    }

    /// `ctrl-i` (Tab in a terminal): one jump forward.
    pub(crate) fn jump_forward(&mut self) {
        let Some(pos) = self.jumplist_future.pop() else {
            self.message = "at newest jump".into();
            return;
        };
        self.jumplist_past.push((self.current, self.cursor));
        self.jump_to(pos);
    }

    /// Land on a jumplist position: switch buffer when it still exists,
    /// skip the entry when its buffer is gone.
    fn jump_to(&mut self, (buffer, offset): (usize, usize)) {
        if buffer >= self.buffers.len() {
            return; // the buffer is closed; the entry dies quietly
        }
        if buffer != self.current {
            self.current = buffer;
            self.touch_mru(buffer);
            self.discover_git();
        }
        self.cursor = self
            .buf()
            .clamp_boundary(offset.min(self.buf().len_bytes()));
        self.clamp_cursor();
        self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn jumplist_walks_back_and_forward_across_buffers() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("j"); // cursor on line 2
        e.push_jump();
        e.feed_text("G"); // last line
        assert_eq!(e.buf().line_of(e.cursor), 2);
        e.jump_back();
        assert_eq!(e.buf().line_of(e.cursor), 1, "ctrl-o back to line 2");
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.cursor), 2, "ctrl-i forward again");
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
        assert_eq!(e.buf().line_of(e.cursor), 1, "landed on the match");
        e.feed(crate::editor::Key::CtrlO);
        assert_eq!(e.buf().line_of(e.cursor), 0, "ctrl-o back to the top");
        e.feed(crate::editor::Key::Tab); // ctrl-i in a terminal
        assert_eq!(e.buf().line_of(e.cursor), 1, "ctrl-i forward again");
    }

    #[test]
    fn search_lands_with_jump_recorded() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo hone\nthree\n"));
        e.feed_text("/hone\r");
        assert_eq!(e.buf().line_of(e.cursor), 1, "landed on the match");
        assert_eq!(e.jumplist_past.len(), 1, "the jump was recorded");
    }
}
