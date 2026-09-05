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

    /// `ctrl-o`: one jump back.
    pub(crate) fn jump_back(&mut self) {
        if self.jumplist_past.is_empty() {
            self.message = "no jumps".into();
            return;
        }
        self.jumplist_future.push((self.current(), self.head()));
        let Some(pos) = self.jumplist_past.pop() else {
            self.jumplist_future.pop();
            self.message = "no jumps".into();
            return;
        };
        self.jump_to(pos);
    }

    /// `ctrl-i` (Tab in a terminal): one jump forward.
    pub(crate) fn jump_forward(&mut self) {
        let Some(pos) = self.jumplist_future.pop() else {
            self.message = "at newest jump".into();
            return;
        };
        self.jumplist_past.push((self.current(), self.head()));
        self.jump_to(pos);
    }

    /// Land on a jumplist position: switch document when it still
    /// exists, skip the entry when its document is gone (generational
    /// id: no index shift, no aliasing — 0014 wave 2).
    fn jump_to(&mut self, (buffer, offset): (strop_core::id::DocumentId, usize)) {
        if self.docs.get(buffer).is_none() {
            return; // the document is closed; the entry dies quietly
        }
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
}
