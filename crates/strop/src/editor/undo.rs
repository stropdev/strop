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
        let origin = self.current();
        let name = self
            .buf()
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
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
        self.push_jump(); // opening the browser is a jumplist entry
        let mut buf = Buffer::from_text(&text);
        buf.readonly = true;
        buf.name = Some("undo tree".into());
        let id = self.docs.insert(Document::output(buf));
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
        // land on the current revision's row
        if let Some(line) = rows.iter().position(|r| r.is_current) {
            self.set_head(self.buf().line_start(line + 1));
        }
        self.undo_browser = Some(UndoBrowser {
            browser: self.current(),
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
        self.view_mut().doc = browser;
        self.close_buffer(true); // browser closes; origin keeps its id
        self.view_mut().doc = origin;
        let Some(ops) = ops else { return };
        let at = ops.iter().map(|e| e.at).min().unwrap_or(0);
        self.buf_mut().apply_history(ops.clone());
        self.bridge_applied_ops(&ops);
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
        if self.current() != ub.browser {
            return false;
        }
        if key == Key::Enter {
            self.undo_tree_jump();
            return true;
        }
        false
    }
}

impl Editor {
    /// One undo unit per command (change ops hold the transaction open
    /// through the insert session — vim groups `ci[foo<esc>` as one `u`).
    pub(crate) fn tx_begin(&mut self) {
        self.buf_mut().history.begin();
    }

    pub(crate) fn tx_commit(&mut self) {
        self.buf_mut().history.commit();
        self.bridge_edits_to_tree();
        // 0020 §14 + 0023: every anchor of this document maps through the
        // transaction — marks, jumplists, and the OTHER panes' cursors.
        // The active pane's selections are each command's own business.
        let Some(all_ops) = self.buf().history.last_committed_ops() else {
            return;
        };
        // the watermark keys on (document, history node) — one op maps
        // anchors EXACTLY once, even when a revision spans several
        // commits (o/O's opening newline + the insert session), and
        // equal depths across documents never collide
        let mark = (self.current(), self.buf().history.depth());
        let skip = match self.anchor_map_mark {
            Some(((d, depth), n)) if (d, depth) == mark => n.min(all_ops.len()),
            _ => 0,
        };
        if skip == all_ops.len() {
            return;
        }
        self.anchor_map_mark = Some((mark, all_ops.len()));
        self.map_anchors_for_current(&all_ops, skip);
    }

    /// Map this document's anchors through a committed op set. Called by
    /// tx_commit (with the watermark's skip) and by undo/redo (inverse
    /// ops map anchors the same way — 0023).
    fn map_anchors_for_current(&mut self, ops: &[strop_core::history::Edit], skip: usize) {
        let ops = ops.iter().skip(skip);
        let doc = self.current();
        let map_one = |mut pos: usize| -> usize {
            for op in ops.clone() {
                let len = op.text.len();
                match op.kind {
                    strop_core::history::EditKind::Insert => {
                        if pos > op.at {
                            pos += len;
                        }
                    }
                    strop_core::history::EditKind::Delete => {
                        if pos > op.at + len {
                            pos -= len;
                        } else if pos > op.at {
                            pos = op.at;
                        }
                    }
                }
            }
            pos
        };
        for (d, m) in self.marks.values_mut() {
            if *d == doc {
                *m = map_one(*m);
            }
        }
        for (d, j) in self
            .jumplist_past
            .iter_mut()
            .chain(self.jumplist_future.iter_mut())
        {
            if *d == doc {
                *j = map_one(*j);
            }
        }
        let active = self.active_pane;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            if pane.doc != doc || i == active {
                continue;
            }
            let primary = pane.sels.primary();
            pane.sels
                .stretch_primary(map_one(primary.anchor), map_one(primary.head));
            let extras: Vec<usize> = pane
                .sels
                .extra_heads()
                .iter()
                .map(|s| map_one(s.head))
                .collect();
            pane.sels.set_extras(extras);
        }
    }

    /// 0022 §1: the parse tree tracks each committed edit — the bridge
    /// is a cheap pointer walk at commit time; the reparse stays lazy.
    fn bridge_edits_to_tree(&mut self) {
        let doc = self.cur_mut();
        if let (Some(h), Some(ops)) = (
            doc.highlighter.as_mut(),
            doc.buf.history.last_committed_ops(),
        ) {
            let revision = doc.buf.epoch;
            h.apply_edits(&Self::ts_edits(&doc.buf, &ops), revision);
        }
    }

    /// Drop the kept tree when exact coordinates aren't computable.
    pub(crate) fn invalidate_syntax_tree(&mut self) {
        if let Some(h) = self.cur_mut().highlighter.as_mut() {
            h.invalidate();
        }
    }

    /// Bridge the last-applied ops (undo/redo included) to the tree.
    pub(crate) fn bridge_applied_ops(&mut self, ops: &[strop_core::history::Edit]) {
        let doc = self.cur_mut();
        if let Some(h) = doc.highlighter.as_mut() {
            let revision = doc.buf.epoch;
            h.apply_edits(&Self::ts_edits(&doc.buf, ops), revision);
        }
    }

    fn ts_edits(
        buf: &strop_core::Buffer,
        ops: &[strop_core::history::Edit],
    ) -> Vec<tree_sitter::InputEdit> {
        ops.iter()
            .map(|op| {
                let e = buf.input_edit_of(op);
                tree_sitter::InputEdit {
                    start_byte: e.start_byte,
                    old_end_byte: e.old_end_byte,
                    new_end_byte: e.new_end_byte,
                    start_position: tree_sitter::Point {
                        row: e.start_point.0,
                        column: e.start_point.1,
                    },
                    old_end_position: tree_sitter::Point {
                        row: e.old_end_point.0,
                        column: e.old_end_point.1,
                    },
                    new_end_position: tree_sitter::Point {
                        row: e.new_end_point.0,
                        column: e.new_end_point.1,
                    },
                }
            })
            .collect()
    }

    /// `u`: undo one revision. Readonly buffers never record.
    pub(crate) fn undo(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        match self.buf_mut().history.undo_ops() {
            Some(ops) => {
                // vim lands the cursor at the *start* of the undone
                // change; undo ops replay in reverse record order, so
                // first() is the tail of the change — take the minimum
                let start = ops.iter().map(|e| e.at).min().unwrap_or(0);
                self.buf_mut().apply_history(ops.clone());
                self.bridge_applied_ops(&ops);
                // undo moves anchors too (0023: marks follow their text)
                self.map_anchors_for_current(&ops, 0);
                self.set_head(start);
                self.clamp_cursor();
                self.flash(strop_core::Range::charwise(self.head(), self.head()));
            }
            None => self.message = "already at oldest change".into(),
        }
    }

    /// `ctrl-r`: redo along the last-visited branch.
    pub(crate) fn redo(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        match self.buf_mut().history.redo_ops() {
            Some(ops) => {
                // cursor after the redone text for inserts, at the start
                // of the redone deletion for deletes
                let at = ops
                    .last()
                    .map(|e| match e.kind {
                        strop_core::history::EditKind::Insert => e.at + e.text.len(),
                        strop_core::history::EditKind::Delete => e.at,
                    })
                    .unwrap_or(0);
                self.buf_mut().apply_history(ops.clone());
                self.bridge_applied_ops(&ops);
                self.map_anchors_for_current(&ops, 0);
                self.set_head(at);
                self.clamp_cursor();
                self.flash(strop_core::Range::charwise(self.head(), self.head()));
            }
            None => self.message = "nothing to redo".into(),
        }
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
