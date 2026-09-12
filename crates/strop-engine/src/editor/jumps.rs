//! vim's jumplist: `ctrl-o` back, `ctrl-i` forward (in a terminal
//! ctrl-i *is* Tab — same as vim). past/future stacks: a new jump
//! truncates the future, like vim.
//!
//! 0051 §7 (R07): entries are full navigation/view records, not bare
//! positions — `ctrl-o`/`ctrl-i` put back the caret, the selection,
//! the viewport top and the horizontal origin the user actually saw.
//! New-destination landings (definition/grep/picker/mark/collection
//! jumps) go through [`Editor::place_jump_target`]: one explicit
//! policy — a comfortably visible target keeps its view, anything
//! else lands near vertical center.

use strop_core::id::{DisplayColumn, DocumentId};

use super::Editor;

/// One jumplist entry (0051 §7): the whole navigation/view record.
///
/// `view_top` is stored as the BYTE OFFSET of the first visible
/// line's start, not a line number: the edit journal remaps byte
/// offsets pointwise (transact.rs), so a recorded view survives edits
/// above it exactly the way the caret does. `jump_to` converts back
/// to a line and clamps against the live geometry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JumpRecord {
    pub document: DocumentId,
    /// Primary caret (selection head), byte offset.
    pub offset: usize,
    /// First visible line, as the byte offset of its start.
    pub view_top: usize,
    /// Horizontal display-cell origin.
    pub hscroll: DisplayColumn,
    /// Selection anchor; equals `offset` for a bare caret.
    pub anchor: usize,
    pub extras: Vec<strop_core::selection::Selection>,
}

impl Editor {
    /// Every retained source view uses the same byte-domain remapping.
    pub(crate) fn map_navigation_records(
        &mut self,
        document: DocumentId,
        map: impl Fn(usize) -> usize,
    ) {
        for record in self
            .jumplist_past
            .iter_mut()
            .chain(self.jumplist_future.iter_mut())
            .chain(
                self.docs
                    .values_mut()
                    .filter_map(|doc| doc.return_point_mut()),
            )
            .chain(
                self.review
                    .replace_context
                    .iter_mut()
                    .map(|context| &mut context.origin),
            )
        {
            if record.document == document {
                record.offset = map(record.offset);
                record.anchor = map(record.anchor);
                record.view_top = map(record.view_top);
                for selection in &mut record.extras {
                    selection.anchor = map(selection.anchor);
                    selection.head = map(selection.head);
                }
            }
        }
    }

    /// The record `push_jump` stores and `jump_back`/`jump_forward`
    /// swap between stacks: the live caret, selection, viewport and
    /// horizontal origin, captured BEFORE the jump happens.
    pub(crate) fn jump_record(&self) -> JumpRecord {
        let view = self.view();
        let top_line = view.view_top.min(self.buf().len_lines().saturating_sub(1));
        JumpRecord {
            document: view.doc,
            offset: self.head(),
            view_top: self.buf().line_start(top_line),
            hscroll: view.hscroll,
            anchor: self.sels().primary().anchor,
            extras: self.sels().extra_heads().to_vec(),
        }
    }

    /// Record the current position before a jump-causing action
    /// (gd, gg, G, %, /, n, marks, buffer switches, dives). Consecutive
    /// duplicates don't pile up: re-recording the same spot refreshes
    /// the view the entry carries instead of stacking a twin.
    pub(crate) fn push_jump(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        let record = self.jump_record();
        match self.jumplist_past.last_mut() {
            Some(last) if last.document == record.document && last.offset == record.offset => {
                *last = record;
            }
            _ => self.jumplist_past.push(record),
        }
        self.jumplist_future.clear(); // a new jump truncates the forward path
    }
    /// `ctrl-o`: one jump back. Entries whose document was closed die
    /// here, not in `jump_to` — skipping there would leave the current
    /// position stranded on the future stack.
    pub(crate) fn jump_back(&mut self) {
        while let Some(record) = self.jumplist_past.pop() {
            if self.docs.get(record.document).is_none() {
                continue; // the document is closed; the entry dies quietly
            }
            let here = self.jump_record();
            self.jumplist_future.push(here);
            self.jump_to(record);
            return;
        }
        self.message = "no jumps".into();
    }

    /// `ctrl-i` (Tab in a terminal): one jump forward. Dead entries are
    /// skipped the same way as `jump_back`.
    pub(crate) fn jump_forward(&mut self) {
        while let Some(record) = self.jumplist_future.pop() {
            if self.docs.get(record.document).is_none() {
                continue;
            }
            let here = self.jump_record();
            self.jumplist_past.push(here);
            self.jump_to(record);
            return;
        }
        self.message = "at newest jump".into();
    }

    /// Land on a jumplist record: switch document when needed, then
    /// restore the recorded caret, selection, viewport and horizontal
    /// origin. Callers (`jump_back`/`jump_forward`) have already
    /// dropped entries whose document is gone.
    pub(crate) fn jump_to(&mut self, record: JumpRecord) {
        debug_assert!(
            self.docs.get(record.document).is_some(),
            "jump_to: dead entry"
        );
        if record.document != self.current() {
            self.switch_to(record.document);
            self.discover_git();
        }
        let len = self.buf().len_bytes();
        let head = self.buf().clamp_boundary(record.offset.min(len));
        let anchor = self.buf().clamp_boundary(record.anchor.min(len));
        self.sels_mut().stretch_primary(anchor, head);
        let extras: Vec<_> = record
            .extras
            .into_iter()
            .map(|selection| strop_core::selection::Selection {
                anchor: self.buf().clamp_boundary(selection.anchor.min(len)),
                head: self.buf().clamp_boundary(selection.head.min(len)),
            })
            .collect();
        self.sels_mut().set_extra_selections(extras);
        self.clamp_cursor();
        // The recorded view comes back; a view that no longer shows
        // the caret (edits or a resize hollowed it out) is no longer
        // meaningful — fall back to centering the target (0051 §7).
        self.view_mut().hscroll = record.hscroll;
        let rows = self.view_rows();
        if rows > 0 {
            // same clamp `zz` uses: never scroll past the content end
            let max_top = (self.buf().last_content_line() + 1).saturating_sub(rows);
            let top = self
                .buf()
                .line_of(record.view_top.min(self.buf().len_bytes()))
                .min(max_top);
            self.view_mut().view_top = top;
            let line = self.buf().line_of(self.head());
            if !(top..top + rows).contains(&line) {
                self.view_place('z');
            }
        }
        self.flash(strop_core::Range::charwise(self.head(), self.head()));
    }

    /// Land on a NEW jump destination (picker jumplist menu): switch
    /// document when needed, then place the target deliberately.
    pub(crate) fn jump_land(&mut self, document: DocumentId, offset: usize) {
        debug_assert!(self.docs.get(document).is_some(), "jump_land: dead entry");
        if document != self.current() {
            self.switch_to(document);
            self.discover_git();
        }
        self.set_head(
            self.buf()
                .clamp_boundary(offset.min(self.buf().len_bytes())),
        );
        self.clamp_cursor();
        self.place_jump_target();
        self.flash(strop_core::Range::charwise(self.head(), self.head()));
    }

    /// The one jump-landing placement policy (0051 §7): a target
    /// already comfortably visible keeps its view — no jitter; a
    /// target near the edge or off-screen lands near vertical center,
    /// respecting file bounds (the `zz` geometry, which never scrolls
    /// past the content end). Ordinary motions, `zt`/`zz`/`zb` and
    /// passive updates never come through here.
    pub(crate) fn place_jump_target(&mut self) {
        let rows = self.view_rows();
        if rows == 0 {
            return; // no committed geometry yet; the render loop settles
        }
        let line = self.buf().line_of(self.head());
        let top = self.view_top();
        // A modest context margin: inside it the current view is
        // already a good answer, so leave it alone.
        let margin = (rows / 8).clamp(1, 4).min(rows.saturating_sub(1) / 2);
        if (top + margin..top + rows - margin).contains(&line) {
            return;
        }
        self.view_place('z');
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

    /// A tall document with the caret parked at `line`, the viewport
    /// top at `top` and a horizontal origin of `hscroll` cells.
    fn viewed_editor(lines: usize, line: usize, top: usize, hscroll: usize) -> Editor {
        let text: String = (0..lines).fold(String::new(), |mut acc, i| {
            acc.push_str(&format!("line {i:03} with some text in it\n"));
            acc
        });
        let mut e = Editor::new(Buffer::from_text(&text));
        e.scroll_to_cursor(20); // commit a 20-row geometry
        let at = e.buf().line_start(line) + 2;
        e.sels_mut().collapse_primary(at); // a real caret: anchor == head
        e.view_mut().view_top = top;
        e.view_mut().hscroll = DisplayColumn::new(hscroll);
        e
    }

    #[test]
    fn jump_restores_caret_view_hscroll_and_selection() {
        let mut e = viewed_editor(200, 60, 55, 7);
        // a stretched selection around the caret, like visual mode left
        let (anchor_at, head_at) = (e.buf().line_start(60) + 5, e.head());
        e.sels_mut().stretch_primary(anchor_at, head_at);
        let anchor = e.sels().primary().anchor;
        e.push_jump();
        e.feed_text("G"); // jump away: caret, view and origin all move
        e.view_mut().hscroll = DisplayColumn::new(0);
        e.jump_back();
        assert_eq!(e.buf().line_of(e.head()), 60, "caret restored");
        assert_eq!(e.sels().primary().anchor, anchor, "selection restored");
        assert_eq!(e.view_top(), 55, "viewport top restored");
        assert_eq!(
            e.view().hscroll,
            DisplayColumn::new(7),
            "horizontal origin restored"
        );
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.head()), 199, "forward to the jump target");
    }

    #[test]
    fn closing_help_restores_and_remaps_the_complete_source_view() {
        let mut editor = viewed_editor(200, 60, 55, 7);
        let source = editor.current();
        let head = editor.head();
        editor.sels_mut().stretch_primary(head + 3, head);
        editor
            .sels_mut()
            .set_extra_selections(vec![strop_core::selection::Selection::cursor(head + 10)]);
        editor.open_help();
        assert!(editor.sels().extra_heads().is_empty());
        editor
            .doc_mut(source)
            .buf
            .edit()
            .insert(0, "new line\n")
            .unwrap();
        editor.feed_text(":q<cr>");
        assert_eq!(editor.current(), source);
        assert_eq!(editor.head(), head + 9);
        assert_eq!(editor.sels().primary().anchor, head + 12);
        assert_eq!(editor.sels().extra_heads()[0].head, head + 19);
        assert_eq!(editor.view_top(), 56);
        assert_eq!(editor.view().hscroll, DisplayColumn::new(7));
    }

    #[test]
    fn jump_restores_view_across_buffers() {
        let mut e = viewed_editor(200, 80, 75, 3);
        e.push_jump();
        let b = e.docs.insert(Document::output(Buffer::from_text("x\ny\n")));
        e.switch_to(b);
        e.jump_back();
        assert_ne!(e.current(), b, "switched back to the first document");
        assert_eq!(e.buf().line_of(e.head()), 80);
        assert_eq!(e.view_top(), 75);
        assert_eq!(e.view().hscroll, DisplayColumn::new(3));
    }

    #[test]
    fn edits_above_the_record_remap_caret_and_view() {
        let mut e = viewed_editor(200, 60, 55, 0);
        e.push_jump();
        e.feed_text("G");
        // insert two lines above the recorded position
        e.feed_text("ggOnew\nmore\u{1b}");
        // the entry rode the edit down: caret, anchor and viewport top
        let record = e.jumplist_past[0].clone();
        assert_eq!(
            e.buf().line_of(record.offset),
            62,
            "the recorded caret rode the edit down"
        );
        assert_eq!(e.buf().line_of(record.anchor), 62, "the anchor rode too");
        assert_eq!(
            e.buf().line_of(record.view_top),
            57,
            "the recorded viewport rode the same edit"
        );
        // and the remapped record still restores cleanly
        e.jump_to(record);
        assert_eq!(e.buf().line_of(e.head()), 62);
        assert_eq!(e.view_top(), 57);
    }

    #[test]
    fn landing_centers_an_offscreen_target() {
        let mut e = viewed_editor(200, 10, 5, 0);
        let target = e.buf().line_start(120);
        e.push_jump();
        e.jump_land(e.current(), target);
        assert_eq!(e.buf().line_of(e.head()), 120, "landed on the target");
        let top = e.view_top();
        let rows = e.view_rows();
        assert!(
            (top + rows / 4..top + rows - rows / 4).contains(&120),
            "target near vertical center: top={top} rows={rows}"
        );
        assert_eq!(top, 110, "zz geometry: 120 + 1 - (20/2 + 1)");
    }

    #[test]
    fn landing_keeps_a_comfortably_visible_target() {
        let mut e = viewed_editor(200, 100, 95, 0);
        let target = e.buf().line_start(105); // comfortably visible: top+2..top+18
        e.push_jump();
        e.jump_land(e.current(), target);
        assert_eq!(e.buf().line_of(e.head()), 105);
        assert_eq!(e.view_top(), 95, "no jitter for an already-visible target");
    }

    #[test]
    fn landing_centers_a_target_at_the_view_edge() {
        let mut e = viewed_editor(200, 100, 95, 0);
        let target = e.buf().line_start(114); // inside the view, outside the margin
        e.push_jump();
        e.jump_land(e.current(), target);
        assert_eq!(e.buf().line_of(e.head()), 114);
        assert_ne!(e.view_top(), 95, "edge targets recenter");
    }

    #[test]
    fn restored_view_invalid_after_resize_falls_back_to_centering() {
        let mut e = viewed_editor(200, 60, 55, 0);
        e.push_jump();
        e.feed_text("G");
        // the pane shrank: the recorded 55..75 window no longer shows
        // a caret on line 60, so the restore centers instead
        e.scroll_to_cursor(4);
        e.jump_back();
        let line = e.buf().line_of(e.head());
        assert_eq!(line, 60, "caret restored");
        let top = e.view_top();
        assert!(
            (top..top + 4).contains(&line),
            "the caret is visible after the restore: top={top} line={line}"
        );
        assert_eq!(top, 58, "zz geometry for 4 rows: 60 + 1 - (4/2 + 1)");
    }
}
