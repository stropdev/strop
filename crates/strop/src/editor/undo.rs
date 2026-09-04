//! The undo-tree browser (`Space u`): the buffer's history tree as a
//! real readonly buffer (0001 §4). Enter restores the revision under
//! the cursor; q closes. State is just the line→revision mapping —
//! the tree itself lives in `strop_core::history::History`.

use strop_core::{Buffer, Range};

use super::{Document, Editor, Key};

/// Live browser state: which buffer is the browser, which buffer it
/// describes, and the revision index per text row (after the header).
pub struct UndoBrowser {
    pub browser: strop_core::id::DocumentId,
    pub origin: strop_core::id::DocumentId,
    /// revision index per browser line (line 0 is the header).
    pub row_rev: Vec<Option<usize>>,
}

impl Editor {
    /// `Space u`: open the undo tree of the current buffer.
    pub(crate) fn open_undo_tree(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let rows = self.buf().history.tree_rows();
        if rows.is_empty() {
            self.message = "no undo history".into();
            return;
        }
        let origin = self.current;
        let name = self
            .buf()
            .path
            .clone()
            .unwrap_or_else(|| "[scratch]".into());
        let mut text = format!("undo tree — {name}   (enter: restore · q: close)\n");
        let mut row_rev = vec![None];
        for r in &rows {
            let indent = "  ".repeat(r.depth.saturating_sub(1));
            let cur = if r.is_current { " ← current" } else { "" };
            let branch = if r.branches { "⑂ " } else { "" };
            text.push_str(&format!("{}* {}#{}{cur}\n", indent, branch, r.index));
            row_rev.push(Some(r.index));
        }
        self.drop_stale_scratch();
        self.push_jump(); // opening the browser is a jumplist entry
        let mut buf = Buffer::from_text(&text);
        buf.readonly = true;
        buf.name = Some("undo tree".into());
        let id = self.docs.insert(Document {
            buf,
            highlighter: None,
            surface: None,
        });
        self.current = id;
        self.touch_mru(id);
        self.set_head(0);
        self.view_top = 0;
        // land on the current revision's row
        if let Some(line) = rows.iter().position(|r| r.is_current) {
            self.set_head(self.buf().line_start(line + 1));
        }
        self.undo_browser = Some(UndoBrowser {
            browser: self.current,
            origin,
            row_rev,
        });
    }

    /// Enter in the browser: restore the row's revision into the origin
    /// buffer, then close back to it.
    fn undo_tree_jump(&mut self) {
        let Some(ub) = &self.undo_browser else { return };
        let line = self.buf().line_of(self.head());
        let Some(Some(rev)) = ub.row_rev.get(line).copied() else {
            return;
        };
        let (browser, origin) = (ub.browser, ub.origin);
        let ops = self.doc_mut(origin).buf.history.ops_to(rev);
        self.undo_browser = None;
        self.current = browser;
        self.close_buffer(true); // browser closes; origin keeps its id
        self.current = origin;
        let Some(ops) = ops else { return };
        let at = ops.iter().map(|e| e.at).min().unwrap_or(0);
        self.buf_mut().apply_history(ops);
        self.set_head(self.buf().clamp_boundary(at.min(self.buf().len_bytes())));
        self.clamp_cursor();
        self.flash(Range::charwise(self.head(), self.head()));
        self.message = format!("restored revision {rev} — u to walk back");
    }

    /// Browser key intercept: Enter restores; everything else falls
    /// through to the readonly-surface motions. Returns true when the
    /// browser consumed the key.
    pub(crate) fn feed_undo_browser(&mut self, key: Key) -> bool {
        let Some(ub) = &self.undo_browser else {
            return false;
        };
        // stale state guard: the browser buffer must still be current
        let alive = self
            .docs
            .get(ub.browser)
            .is_some_and(|d| d.buf.name.as_deref() == Some("undo tree"));
        if !alive {
            self.undo_browser = None;
            return false;
        }
        if self.current != ub.browser {
            return false;
        }
        if key == Key::Enter {
            self.undo_tree_jump();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn browser_lists_tree_and_restores_branch() {
        let mut e = Editor::new(Buffer::from_text("a\n"));
        e.feed_text("ob\x1b"); // revision 1: append line "b"
        e.feed_text("u");
        e.feed_text("oc\x1b"); // revision 2: fork — append line "c"
        e.feed_text(" u");
        let br = e.undo_browser.as_ref().expect("browser open");
        assert_eq!(e.buf().name.as_deref(), Some("undo tree"));
        assert_eq!(br.row_rev.len(), 3); // header + 2 revisions
                                         // cursor starts on the current revision (#2, the "c" branch)
        e.feed_text("j"); // down to revision 1 (the "b" branch)
        e.feed(crate::editor::Key::Enter);
        assert_eq!(e.buf().name.as_deref(), None, "back on the file");
        assert_eq!(e.buf().rope.to_string(), "a\nb\n");
        // and the restored state keeps its history: u walks back to "a"
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "a\n");
    }

    #[test]
    fn browser_q_closes_back_to_origin() {
        let mut e = Editor::new(Buffer::from_text("a\n"));
        e.feed_text("ob\x1b");
        e.feed_text(" u");
        assert!(e.undo_browser.is_some());
        e.feed_text("q");
        assert_eq!(e.buf().name.as_deref(), None);
        assert_eq!(e.buf().rope.to_string(), "a\nb\n");
    }
}
