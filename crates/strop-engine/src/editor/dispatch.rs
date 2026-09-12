//! One typed input owner, computed from state (0028 P2): the replacement
//! for a hand-ordered condition chain whose branch positions were
//! load-bearing and invisible. Each owner declares its modal policy as
//! data; transient overlays forward keys in insert mode after dismissing
//! themselves (the 0.20.1 swallowed-keystroke fix), modal owners consume.

use super::{Editor, Key, Mode};

/// Who consumes the next key. Priority is fixed by `input_owner`'s
/// evaluation order: pending fields, pickers, transient cards, document.
/// A late hover/blame reply never steals a modal field's input or paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOwner {
    /// A live `: / ? |` line.
    Pending,
    /// The LSP hover card — a transient overlay.
    HoverCard,
    /// The blame card or its in-flight request — a transient overlay.
    BlameCard,
    /// An open picker (its fields are their own modal input).
    Picker,
    /// The undo-tree browser.
    UndoBrowser,
    /// The document itself: mode dispatch (insert/visual/normal).
    Document,
}

impl InputOwner {
    /// Transient overlays dismiss themselves and FORWARD the key in
    /// insert mode: an async reply can land mid-insert, and the next
    /// keystroke (Esc most painfully) must still land. Everywhere else
    /// overlays consume the key that dismisses them.
    fn forwards_in_insert(self) -> bool {
        matches!(self, Self::HoverCard | Self::BlameCard)
    }
}

impl Editor {
    /// The typed owner of the next key, computed from state.
    pub fn input_owner(&self) -> InputOwner {
        if self.pending.is_active() {
            return InputOwner::Pending;
        }
        if self.picker_open() {
            return InputOwner::Picker;
        }
        if self.hover_card.is_some() {
            return InputOwner::HoverCard;
        }
        if self.blame_card.is_some() || self.card_request.is_some() {
            return InputOwner::BlameCard;
        }
        // The undo browser owns keys only while its buffer is live and
        // current (the stale-state guard stays in feed_undo_browser).
        if self.undo_browser.as_ref().is_some_and(|ub| {
            self.docs
                .get(ub.browser)
                .is_some_and(|d| d.buf.name.as_deref() == Some("undo tree"))
        }) {
            return InputOwner::UndoBrowser;
        }
        InputOwner::Document
    }

    /// Dispatch one key to its owner. Modal policy is the owner's data:
    /// transient overlays forward in insert mode; everything else
    /// consumes. The document owner is the mode machine.
    pub(super) fn dispatch_owned(&mut self, key: Key) {
        let owner = self.input_owner();
        if self.mode == Mode::Insert && owner.forwards_in_insert() {
            self.dismiss_overlay(owner);
            self.feed_document(key);
            return;
        }
        match owner {
            InputOwner::Pending => self.feed_pending(key),
            InputOwner::HoverCard => {
                if key == Key::Enter {
                    self.open_hover_document();
                } else {
                    self.hover_card = None;
                }
            }
            InputOwner::BlameCard => {
                let had_card = self.blame_card.is_some();
                if let Some(card) = self.dismiss_card_authority() {
                    if key == Key::Enter {
                        self.open_log_at(&card.sha);
                    }
                }
                if !had_card {
                    // only an in-flight request was cancelled above; the
                    // key still belongs to whatever owns it next
                    self.dispatch_owned(key);
                }
            }
            InputOwner::Picker => self.feed_picker(key),
            InputOwner::UndoBrowser => {
                if !self.feed_undo_browser(key) {
                    self.feed_document(key);
                }
            }
            InputOwner::Document => self.feed_document(key),
        }
    }

    /// Dismiss a transient overlay without consuming the key (insert-mode
    /// forwarding path).
    fn dismiss_overlay(&mut self, owner: InputOwner) {
        match owner {
            InputOwner::HoverCard => self.hover_card = None,
            InputOwner::BlameCard => {
                let _ = self.dismiss_card_authority();
            }
            _ => {}
        }
    }

    /// The mode machine: insert/visual/normal dispatch.
    fn feed_document(&mut self, key: Key) {
        match self.mode {
            Mode::Insert => self.feed_insert(key),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.feed_visual(key),
            Mode::Normal => self.feed_normal(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn late_cards_cannot_consume_modal_field_input() {
        let mut editor = Editor::new(Buffer::from_text("x\n"));
        editor.open_picker(strop_picker::Kind::RemoteAddress);
        editor.hover_card = Some("late docs".into());
        editor.feed_text("host");
        assert_eq!(editor.picker.as_ref().unwrap().picker.input.text, "host");
    }

    #[test]
    fn overlays_forward_in_insert_and_consume_in_normal() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("i");
        e.hover_card = Some("late docs".into());
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.mode, Mode::Normal, "esc landed through the overlay");
        assert!(e.hover_card.is_none());
        e.hover_card = Some("docs".into());
        e.feed(crate::editor::Key::Char('j'));
        assert!(e.hover_card.is_none(), "normal mode: the key dismisses");
    }
}
