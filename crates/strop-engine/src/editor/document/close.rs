//! Closing documents: `:q`'s guards and landing ([`Editor::close_buffer`]),
//! the teardown both it and a collection's released loans share
//! ([`Editor::release_document`], 0101 §6.2), and vim's [No Name] rule.

use strop_core::id::DocumentId;

use super::super::jumps::JumpRecord;
use super::super::Editor;

/// What a teardown hands back to the close that asked for it.
pub(crate) struct Released {
    /// Where a closing surface returns the cursor (0051 §7 R07).
    pub return_to: Option<JumpRecord>,
    /// A change review whose document this was.
    pub review: Option<crate::editor::changes::review::ChangeProposal>,
    /// Collections that lost excerpts of this document: re-rendered once
    /// the panes are rebound.
    pub affected: Vec<DocumentId>,
}

impl Editor {
    /// vim's [No Name] rule: the untouched initial scratch buffer is
    /// replaced by the first real thing you open. `replacement` is the
    /// document taking over (0023 §1): panes on the scratch rebind to
    /// it FIRST — the drop used to strand them (the :vs crash probe).
    pub(crate) fn drop_stale_scratch(&mut self, replacement: DocumentId) {
        // find the pristine scratch (pathless, untouched) wherever it is
        // — the replacement exists by now, so len() is no longer the
        // signal (0023: it must fire AFTER the insert so panes can rebind)
        let scratch = self.docs.iter().find_map(|(id, d)| {
            let b = &d.buf;
            (b.path.is_none()
                && !b.is_dirty()
                && b.len_bytes() == 0
                && b.name.is_none()
                && id != replacement)
                .then_some(id)
        });
        let Some(scratch) = scratch else {
            return;
        };
        if self
            .pending
            .prompt()
            .is_some_and(|prompt| prompt.origin().pane.doc == scratch)
        {
            self.cancel_pending();
        }
        for pane in &mut self.panes {
            if pane.doc == scratch {
                pane.doc = replacement;
            }
        }
        self.lsp_close_document(scratch);
        self.docs.remove(scratch);
        self.mru.retain(|&x| x != scratch);
        if self.view().doc == scratch {
            self.view_mut().doc = replacement;
        }
    }

    /// The teardown of one document, and nothing else: no guard, no
    /// landing, no cursor moves (0101 §6.2). Every service that keeps
    /// something per document lets go of it here, so `:q` and a released
    /// collection loan close a document the same way. The caller has
    /// already moved every pane off it — except `close_buffer`, which
    /// rebinds the panes from the returned record.
    pub(crate) fn release_document(&mut self, closed: DocumentId) -> Released {
        self.forget_terminal_document(closed);
        let mut affected = Vec::new();
        for (id, collection) in &mut self.collections {
            let before = collection.excerpts.len();
            collection
                .excerpts
                .retain(|excerpt| excerpt.source != closed);
            if collection.excerpts.len() != before {
                affected.push(*id);
            }
        }
        self.drop_remote_write(closed);
        self.analysis
            .forget(super::super::analysis::AnalysisTarget::Document(closed));
        self.cancel_directory_filter(closed);
        self.lsp_close_document(closed);
        self.shell_document_closed(closed);
        self.revoke_git_requests_for(closed);
        self.blame_gutters.remove(&closed);
        self.collections.remove(&closed);
        let review = self.review.forget(closed);
        self.filesystem_forget_view(closed);
        let return_to = self
            .docs
            .remove(closed)
            .and_then(|document| document.return_point().cloned());
        self.mru.retain(|&x| x != closed);
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        crate::editor::git_memory::graph::document_closed(self);
        Released {
            return_to,
            review,
            affected,
        }
    }

    /// Close the current document; quits when the last one closes.
    /// Returns false when unsaved changes block the close. Generational
    /// ids mean no reindexing anywhere (0014 wave 2).
    pub fn close_buffer(&mut self, force: bool) -> bool {
        self.remember_directory_view();
        // `:q` on a running terminal hides it (Neovim: the job keeps
        // running, `Space b` returns to it); `:bd!` is the kill (0090 §4).
        // Only the last buffer refuses, and `:q!` there stops and quits.
        if self
            .terminal_phase(self.current())
            .is_some_and(strop_terminal::model::Phase::live)
        {
            if let Some(next) = self
                .mru
                .iter()
                .copied()
                .find(|id| *id != self.current() && self.docs.get(*id).is_some())
            {
                self.switch_to(next);
                self.mode = super::super::Mode::Normal;
                self.message = "terminal hidden and still running; Space b returns to it".into();
                return true;
            }
            if force {
                self.stop_all_terminals();
                self.should_quit = true;
                return true;
            }
            self.message = "terminal is running — :q! stops it and quits, :bd! kills it".into();
            return false;
        }
        if !force && self.docs.len() == 1 && self.filesystem.unconfirmed() > 0 {
            self.message =
                "filesystem outcomes are unconfirmed; :fs verify before quitting, or :q! to force"
                    .into();
            return false;
        }
        // A collection's dirty bit is presentation state (0049 §5): the
        // sources hold the real unsaved edits — the view always closes.
        let view_only = self.collections.contains_key(&self.current());
        if self.buf().is_dirty() && !force && !view_only {
            self.message = "unsaved changes — :q! to force".into();
            return false;
        }
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
        if self.docs.len() == 1 {
            self.request_session_save();
        }
        let closed = self.current();
        let released = self.release_document(closed);
        if self.docs.is_empty() {
            self.collection_build = None;
            self.panes.clear();
            self.active_pane = 0;
            self.should_quit = true;
        } else {
            let next = self.mru.first().copied().unwrap_or_else(|| {
                self.docs
                    .iter()
                    .next()
                    .map(|(id, _)| id)
                    .expect("docs non-empty")
            });
            for pane in &mut self.panes {
                if pane.doc == closed {
                    pane.doc = next;
                    pane.sels = Default::default();
                    pane.view_top = 0;
                    pane.hscroll = strop_core::id::DisplayColumn::new(0);
                    pane.desired_column = None;
                }
            }
            self.switch_to(next);
            self.set_head(0);
            self.view_mut().view_top = 0;
            self.view_mut().hscroll = strop_core::id::DisplayColumn::new(0);
            // a closing surface hands the cursor and view back to the
            // document it opened from — by id, no index math (0011 §1)
            if let Some(ret) = released
                .return_to
                .filter(|ret| self.docs.get(ret.document).is_some())
            {
                self.jump_to(ret);
            } else if self.doc(next).directory_metadata_ref().is_some() {
                self.restore_directory_view(next);
            }
        }
        for collection in released.affected {
            self.collection_render_view(collection);
        }
        self.lsp_retire_workspace_servers();
        if let Some(review) = released.review.filter(|_| !self.should_quit) {
            self.review_closed(review);
        }
        if view_only && !self.should_quit {
            // The landing above put a file on screen; it is kept. The
            // rest of what this collection loaded goes (0101 §6.1).
            crate::editor::collections::loans::release_unheld(self);
        }
        true
    }
}
