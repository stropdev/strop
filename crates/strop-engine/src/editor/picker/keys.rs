//! Picker key and paste dispatch while a picker is open: every key
//! resolves against the live field state (insert, or the normal-mode
//! field machine), never against the document behind the card.

use super::{Editor, Key, Kind};

impl Editor {
    pub(crate) fn feed_picker(&mut self, key: Key) {
        let Some(glue) = &mut self.picker else {
            return;
        };
        if key != Key::Enter {
            glue.accept_when_ranked = false;
        }
        let search = glue.picker.kind == Kind::Search;
        let replace = search && glue.picker.replacement_visible;
        // the suggestion list owns accept/cancel while open (0051 R02)
        if glue.suggestions.is_some() {
            match key {
                Key::Up => {
                    let list = glue.suggestions.as_mut().unwrap();
                    list.selected = list.selected.saturating_sub(1);
                    return;
                }
                Key::Down | Key::Tab => {
                    let list = glue.suggestions.as_mut().unwrap();
                    list.selected = (list.selected + 1).min(list.items.len().saturating_sub(1));
                    return;
                }
                Key::Enter => {
                    self.accept_suggestion();
                    return;
                }
                Key::Esc => {
                    glue.suggestions = None;
                    return;
                }
                _ => {
                    glue.suggestions = None;
                }
            }
        }
        match key {
            Key::CtrlSpace => {
                self.open_suggestions();
            }
            Key::Esc => {
                if glue.picker.input_normal() {
                    let origin = glue.search.as_ref().map(|context| context.origin.clone());
                    self.close_picker();
                    if let Some(origin) =
                        origin.filter(|origin| self.docs.get(origin.document).is_some())
                    {
                        self.jump_to(origin);
                    }
                } else {
                    glue.field_keys.clear();
                    glue.picker.enter_normal();
                }
            }
            Key::Enter => self.accept_current_picker(),
            Key::Tab | Key::Backtab if replace => {
                if glue.search.as_ref().is_some_and(|context| {
                    context.scope.root.filesystem != strop_workspace::Filesystem::Local
                }) {
                    self.message =
                        "SSH Search is read-only; With and Review are unavailable".into();
                } else {
                    glue.field_keys.clear();
                    glue.picker.toggle_field();
                }
            }
            // ctrl-o: the listed hits become an editable collection (0044).
            Key::CtrlO => self.open_collection_from_picker(),
            Key::CtrlD if search => {
                if glue.picker.toggle_file_excluded() {
                    self.search_intent_changed();
                } else {
                    self.message = "no source match selected".into();
                }
            }
            Key::CtrlX if search => {
                if glue.picker.toggle_excluded() {
                    self.search_intent_changed();
                } else {
                    self.message = "no source match selected".into();
                }
            }
            Key::CtrlD | Key::CtrlX => {}
            Key::Backspace => {
                if glue.picker.input_normal() {
                    // vim: BS in normal mode is h — a pure motion, so
                    // it never notifies the ranking seams
                    let _ = glue
                        .field_keys
                        .feed(glue.picker.active_field(), Key::Char('h'));
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.pop_replace_char();
                    self.search_intent_changed();
                } else {
                    glue.picker.pop_char();
                    self.picker_input_changed();
                }
            }
            Key::CtrlL => self.needs_repaint = true,
            Key::CtrlR if search => self.toggle_search_replacement(),
            Key::CtrlR | Key::CtrlW => {}
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlV | Key::CtrlCaret => {}
            Key::Up => glue.picker.move_by(-1),
            Key::Down => glue.picker.move_by(1),
            Key::Tab => glue.picker.move_by(1),
            Key::Backtab => glue.picker.move_by(-1),
            Key::Left => glue.picker.caret_left(),
            Key::Right => glue.picker.caret_right(),
            // bare j/k walk the result list (a picker, not a buffer);
            // mid-composition they are grammar (dj, 3j as motions)
            Key::Char('j') if glue.picker.input_normal() && glue.field_keys.is_ground() => {
                glue.picker.move_by(1)
            }
            Key::Char('k') if glue.picker.input_normal() && glue.field_keys.is_ground() => {
                glue.picker.move_by(-1)
            }
            Key::Char(c) => {
                if glue.picker.input_normal() {
                    // the field speaks the real vim grammar (0003 §2);
                    // a completed edit notifies exactly once — the
                    // revision diff replaces the old x/X whitelist, so
                    // dw ranks like x and pure motions never rerank
                    let replace_field =
                        replace && glue.picker.field == strop_picker::Field::Replace;
                    let before = glue.picker.active_field().revision();
                    match glue
                        .field_keys
                        .feed(glue.picker.active_field(), Key::Char(c))
                    {
                        super::super::field::FieldReply::Consumed => {
                            if glue.picker.active_field().revision() != before {
                                if replace_field {
                                    self.search_intent_changed();
                                } else {
                                    self.picker_input_changed();
                                }
                            }
                        }
                        super::super::field::FieldReply::Refused(message) => self.message = message,
                    }
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.push_replace_char(c);
                    self.search_intent_changed();
                } else {
                    glue.picker.push_char(c);
                    self.picker_input_changed();
                }
            }
        }
    }

    /// Bracketed paste while a picker is open edits the focused field
    /// (query, replacement or remote address); it never reaches the
    /// document behind the card. Multi-line payloads are rejected with
    /// a message — a dropped keystroke with no feedback reads as a
    /// broken terminal, not as an editor decision.
    pub(crate) fn paste_picker(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let Some(glue) = &mut self.picker else {
            return;
        };
        if text.contains(['\r', '\n']) {
            self.message = "picker input cannot contain a newline".into();
            return;
        }
        if glue.picker.paste(text) {
            self.picker_input_changed();
        } else {
            self.search_intent_changed();
        }
    }
}
