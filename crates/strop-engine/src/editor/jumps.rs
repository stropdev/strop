//! Vim's per-window jumplist: `ctrl-o` walks older origins and `ctrl-i`
//! walks newer ones (in a terminal ctrl-i *is* Tab). New jumps preserve
//! newer history; only the first backward traversal snapshots the live position.
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

pub const JUMP_LIST_CAP: usize = 100;

/// One indexed history per window. `None` is below the newest entry.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JumpList {
    pub entries: Vec<JumpRecord>,
    pub position: Option<usize>,
}

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
            .panes
            .iter_mut()
            .flat_map(|pane| pane.jumps.entries.iter_mut())
            .chain(
                self.docs
                    .values_mut()
                    .filter_map(|doc| doc.return_point_mut()),
            )
            .chain(
                self.picker
                    .iter_mut()
                    .chain(self.retained_search.iter_mut())
                    .filter_map(|glue| glue.search.as_mut())
                    .map(|context| &mut context.origin),
            )
            .chain(
                self.directories
                    .views
                    .values_mut()
                    .map(|saved| &mut saved.record),
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

    /// The live caret, selection, viewport and horizontal origin, captured
    /// immediately before a successful jump.
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

    /// Append an origin without discarding newer entries. Vim retains one
    /// record per document line and bounds each window's history at 100.
    pub(crate) fn push_jump_record(&mut self, record: JumpRecord) {
        let Some(document) = self.docs.get(record.document) else {
            return;
        };
        let line = document
            .buf
            .line_of(record.offset.min(document.buf.len_bytes()));
        let docs = &self.docs;
        let jumps = &mut self.panes[self.active_pane].jumps;
        jumps.entries.retain(|old| {
            old.document != record.document
                || docs.get(old.document).is_none_or(|old_doc| {
                    old_doc.buf.line_of(old.offset.min(old_doc.buf.len_bytes())) != line
                })
        });
        jumps.entries.push(record);
        let excess = jumps.entries.len().saturating_sub(JUMP_LIST_CAP);
        if excess > 0 {
            jumps.entries.drain(..excess);
        }
        jumps.position = None;
        debug_assert!(jumps.entries.len() <= JUMP_LIST_CAP);
    }

    pub(crate) fn push_jump(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        let record = self.jump_record();
        self.push_jump_record(record);
    }

    /// `ctrl-o`: only the first backward step from below the newest entry
    /// snapshots the live position. Traversal itself never records motions.
    pub(crate) fn jump_back(&mut self) {
        if self.view().jumps.entries.is_empty() {
            self.message = "no jumps".into();
            return;
        }
        if self.view().jumps.position.is_none() {
            let here = self.jump_record();
            self.push_jump_record(here);
            let jumps = &mut self.view_mut().jumps;
            jumps.position = Some(jumps.entries.len() - 1);
        }
        loop {
            let index = self.view().jumps.position.unwrap();
            let Some(next) = index.checked_sub(1) else {
                self.message = "no jumps".into();
                return;
            };
            self.view_mut().jumps.position = Some(next);
            let record = self.view().jumps.entries[next].clone();
            if self.docs.get(record.document).is_some() {
                self.jump_to(record);
                return;
            }
        }
    }

    /// `ctrl-i` (Tab in a terminal): advance only the index, restoring the
    /// stored record and skipping dead documents in either direction.
    pub(crate) fn jump_forward(&mut self) {
        let Some(index) = self.view().jumps.position else {
            self.message = "at newest jump".into();
            return;
        };
        let mut next = index + 1;
        while next < self.view().jumps.entries.len() {
            self.view_mut().jumps.position = Some(next);
            let record = self.view().jumps.entries[next].clone();
            if self.docs.get(record.document).is_some() {
                self.jump_to(record);
                return;
            }
            next += 1;
        }
        self.view_mut().jumps.position = None;
        self.message = "at newest jump".into();
    }

    /// Land on a jumplist record: switch document when needed, then
    /// restore the recorded caret, selection, viewport and horizontal
    /// origin. Callers (`jump_back`/`jump_forward`) have already selected a
    /// live entry, skipping dead documents without changing list order.
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
    fn file_window_starts_with_its_initial_file_origin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("initial.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut e = Editor::new(Buffer::open(path).unwrap());
        assert_eq!(e.view().jumps.entries.len(), 1);
        e.feed_text("j");
        e.jump_back();
        assert_eq!(e.buf().line_of(e.head()), 0);
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.head()), 1);
    }

    #[test]
    fn new_jump_preserves_the_forward_path() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\nd\ne\n"));
        e.push_jump(); // line 1
        e.feed_text("jj");
        e.push_jump(); // line 3
        e.feed_text("G"); // line 5
        e.jump_back(); // line 3
        e.set_head(e.buf().line_start(1));
        e.push_jump(); // branch from line 2, retaining lines 3 and 5
        e.feed_text("j"); // line 3
        assert_eq!(e.view().jumps.entries.len(), 4);
        assert!(e.view().jumps.entries.iter().any(|record| {
            record.document == e.current() && e.buf().line_of(record.offset) == 4
        }));
        assert_eq!(e.view().jumps.position, None);
        e.jump_back(); // line 2
        assert_eq!(e.buf().line_of(e.head()), 1);
        e.jump_forward(); // the synthetic line-3 destination
        assert_eq!(e.buf().line_of(e.head()), 2);
    }

    #[test]
    fn search_then_ctrl_o_ctrl_i() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo hone\nthree\n"));
        e.feed_text("/hone\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "landed on the match");
        e.feed(strop_core::frontend_input::Key::CtrlO);
        assert_eq!(e.buf().line_of(e.head()), 0, "ctrl-o back to the top");
        e.feed(strop_core::frontend_input::Key::Tab); // ctrl-i in a terminal
        assert_eq!(e.buf().line_of(e.head()), 1, "ctrl-i forward again");
    }

    #[test]
    fn paragraphs_visible_lines_and_substitutions_record_origins() {
        let mut e = Editor::new(Buffer::from_text("one\n\ntwo\n\nthree\n"));
        e.feed_text("}");
        assert_eq!(e.view().jumps.entries.len(), 1);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[0].offset), 0);
        e.feed_text("}{");
        assert_eq!(e.view().jumps.entries.len(), 3);
        e.feed_text("G");
        e.feed_text("}");
        assert_eq!(
            e.head(),
            e.buf().len_bytes() - 2,
            "with no blank line below, }} goes to the last character"
        );

        let mut e = Editor::new(Buffer::from_text("one\n\ntwo\n\nthree\n"));
        e.scroll_to_cursor(3);
        e.set_head(e.buf().line_start(4));
        let before = e.view().jumps.entries.len();
        e.feed_text("H");
        assert_eq!(e.view().jumps.entries.len(), before + 1);
        e.feed_text("M");
        e.feed_text("L");
        assert_eq!(e.view().jumps.entries.len(), before + 3);
        let before = e.view().jumps.clone();
        e.feed_text("L");
        assert_eq!(e.view().jumps, before, "repeated L stays at the same line");

        let mut e = Editor::new(Buffer::from_text("foo one\nfoo two\n"));
        e.set_head(e.buf().line_start(1));
        e.feed_text(":%s/foo/bar/g\r");
        assert_eq!(e.buf().text().to_string(), "bar one\nbar two\n");
        assert_eq!(e.view().jumps.entries.len(), 1);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[0].offset), 1);
        let before = e.view().jumps.clone();
        e.feed_text(":%s/missing/nope/g\r");
        assert_eq!(e.view().jumps, before);
        e.feed_text(":%s/[/nope/g\r");
        assert_eq!(e.view().jumps, before);
    }

    #[test]
    fn buffer_acceptance_and_alternate_buffer_record_return_journeys() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        let source = e.current();
        e.set_head(e.buf().line_start(1));
        let b = e
            .docs
            .try_insert(Document::new(Buffer::from_text("other\n")))
            .unwrap();
        e.accept_picker(strop_picker::Payload::Buffer(b), None);
        assert_eq!(e.current(), b);
        assert_eq!(e.view().jumps.entries.len(), 1);
        e.jump_back();
        assert_eq!((e.current(), e.head()), (source, 4));

        e.alternate_buffer();
        assert_eq!(e.current(), b);
        e.jump_back();
        assert_eq!((e.current(), e.head()), (source, 4));
    }

    #[test]
    fn visible_motions_reveal_the_caret_and_use_the_upper_middle() {
        let text = (0..30).fold(String::new(), |mut text, line| {
            text.push_str(&format!("{line:02}\n"));
            text
        });
        let mut e = Editor::new(Buffer::from_text(&text));
        e.scroll_to_cursor(22);
        e.feed_text("GH");
        assert_eq!(e.buf().line_of(e.head()), 8);
        e.feed_text("M");
        assert_eq!(e.buf().line_of(e.head()), 18);
        e.feed_text("L");
        assert_eq!(e.buf().line_of(e.head()), 29);
        e.feed_text("2");
        e.feed(strop_core::frontend_input::Key::CtrlO);
        assert_eq!(e.buf().line_of(e.head()), 8);
    }

    #[test]
    fn failed_marks_leave_history_untouched() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("G");
        let before = e.view().jumps.clone();
        e.feed_text("`z");
        assert_eq!(e.view().jumps, before);
        assert!(e.message.contains("mark z not set"));
    }

    #[test]
    fn small_motions_during_traversal_do_not_invent_entries() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\nd\ne\n"));
        e.feed_text("G");
        e.jump_back();
        let before = e.view().jumps.clone();
        e.feed_text("jk");
        assert_eq!(e.view().jumps, before);
        e.jump_forward();
        assert_eq!(e.view().jumps.entries.len(), before.entries.len());
    }

    #[test]
    fn same_document_line_deduplicates_and_history_caps_at_one_hundred() {
        let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
        e.set_head(1);
        e.push_jump();
        e.set_head(3);
        e.push_jump();
        assert_eq!(e.view().jumps.entries.len(), 1);
        assert_eq!(e.head(), 3);

        let text = (0..120).fold(String::new(), |mut text, line| {
            text.push_str(&format!("line {line}\n"));
            text
        });
        let mut e = Editor::new(Buffer::from_text(&text));
        for line in 0..120 {
            e.set_head(e.buf().line_start(line));
            e.push_jump();
        }
        assert_eq!(e.view().jumps.entries.len(), JUMP_LIST_CAP);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[0].offset), 20);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[99].offset), 119);
    }

    #[test]
    fn split_copies_history_then_panes_evolve_independently() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\nd\n"));
        e.feed_text("G");
        let original = e.view().jumps.clone();
        e.split_document(true, e.current());
        assert_eq!(e.view().jumps, original);
        e.feed_text("gg");
        assert_ne!(e.view().jumps, original);
        assert_eq!(e.panes[0].jumps, original);
        e.active_pane = 0;
        assert_eq!(e.view().jumps, original);
    }

    #[test]
    fn edits_remap_history_in_nonactive_panes() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
        e.feed_text("G");
        e.split_document(true, e.current());
        let original = e.view().jumps.entries[0].offset;
        e.feed_text("ggOnew\u{1b}");
        assert_eq!(e.panes[0].jumps.entries[0].offset, original + 4);
        assert_eq!(e.panes[1].jumps.entries[0].offset, original + 4);
    }

    #[test]
    fn initial_backward_snapshot_deduplicates_the_current_line() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
        e.feed_text("Ggg");
        e.jump_back();
        assert_eq!(e.buf().line_of(e.head()), 2);
        assert_eq!(e.view().jumps.entries.len(), 2);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[0].offset), 2);
        assert_eq!(e.buf().line_of(e.view().jumps.entries[1].offset), 0);
    }

    #[test]
    fn dead_newer_tail_restores_the_below_newest_boundary() {
        let mut e = Editor::new(Buffer::from_text("a0\na1\na2\n"));
        e.push_jump();
        let b = e
            .docs
            .try_insert(Document::new(Buffer::from_text("b0\nb1\nb2\n")))
            .unwrap();
        e.switch_to(b);
        e.push_jump();
        e.feed_text("j");
        e.push_jump();
        e.feed_text("j");
        e.jump_back();
        e.jump_back();
        e.jump_back();
        assert_eq!(e.current(), e.view().jumps.entries[0].document);
        e.feed_text("j");
        e.docs.remove(b);

        e.jump_forward();
        assert_eq!(e.view().jumps.position, None);
        assert_eq!(e.buf().line_of(e.head()), 1);
        e.jump_back();
        assert_eq!(e.buf().line_of(e.head()), 0);
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.head()), 1);
    }

    #[test]
    fn search_lands_with_jump_recorded() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo hone\nthree\n"));
        e.feed_text("/hone\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "landed on the match");
        assert_eq!(e.view().jumps.entries.len(), 1, "the jump was recorded");
    }

    #[test]
    fn ctrl_o_skips_jumps_into_closed_buffers() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("j"); // line 2
        e.push_jump(); // (docA, line 2)
        e.feed_text("G"); // line 3
        let b = e
            .docs
            .try_insert(Document::output(Buffer::from_text("x\ny\n")))
            .unwrap();
        e.switch_to(b);
        e.push_jump(); // (docB, 0) — dead after the close below
        e.close_buffer(true); // back to docA, cursor at its default first line
        assert_eq!(e.buf().line_of(e.head()), 0);
        e.jump_back();
        // the dead entry is skipped; the LIVE docA entry is the landing
        assert_eq!(e.buf().line_of(e.head()), 1);
        e.jump_forward();
        assert_eq!(e.buf().line_of(e.head()), 0, "forward skips the dead entry");
        e.jump_back();
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
        let b = e
            .docs
            .try_insert(Document::output(Buffer::from_text("x\ny\n")))
            .unwrap();
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
        let record = e.view().jumps.entries[0].clone();
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
