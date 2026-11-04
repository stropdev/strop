//! The built-in palette selector. Config supplies the startup choice;
//! `--theme` overrides it and `:theme` switches only this session.
//! Existing terminals receive an asynchronous palette-default update.
use super::Editor;
use strop_core::theme::{self, ThemeId};
use strop_picker::{Item, Payload, Picker};

impl Editor {
    /// `:theme` opens the selector; `:theme NAME` applies a registered
    /// palette. Unknown names leave the current choice untouched.
    pub(crate) fn theme_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_theme_picker();
            return;
        }
        match ThemeId::from_name(arg) {
            Some(id) => self.apply_theme(id),
            None => {
                self.message = format!(
                    "theme takes {} — not {arg:?}",
                    ThemeId::ALL
                        .iter()
                        .map(|id| id.name())
                        .collect::<Vec<_>>()
                        .join(" or ")
                );
            }
        }
    }

    fn open_theme_picker(&mut self) {
        let items: Vec<Item> = ThemeId::ALL
            .into_iter()
            .map(|id| Item {
                badge: None,
                // Ranking searches only the required human and canonical
                // identities; the selected detail owns the full description.
                // Its match columns stay valid because the label is the prefix.
                text: format!("{} — {}", id.label(), id.name()),
                payload: Payload::ThemeChoice(id),
            })
            .collect();
        let picker = Picker::new(strop_picker::Kind::Theme, items, false);
        self.set_picker(super::picker::PickerGlue::diagnostics(picker));
    }

    /// The one mutation path: selector rows and the direct argument
    /// land here alike.
    pub(crate) fn apply_theme(&mut self, id: ThemeId) {
        theme::set_current(id);
        self.message = match self.refresh_terminal_themes() {
            Ok(()) => format!("theme: {} palette", id.name()),
            Err(error) => format!("theme: {}; terminal refresh failed: {error}", id.name()),
        };
    }
}
