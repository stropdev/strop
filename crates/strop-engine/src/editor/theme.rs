//! The `:theme` palette selector (0.42.0): two compiled-in palettes,
//! switched live. Render and the terminal read `theme::current()` per
//! frame, so the switch paints on the next frame — no invalidation,
//! no config file (0005 owns that when it lands).
use super::Editor;
use strop_core::theme::{self, ThemeId};
use strop_picker::{Item, Payload, Picker};

impl Editor {
    /// `:theme` with no argument opens the selector; `:theme dark` /
    /// `:theme light` switches directly. Anything else is a named
    /// refusal listing the real choices.
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
        let current = theme::current_id();
        let items: Vec<Item> = ThemeId::ALL
            .into_iter()
            .map(|id| Item {
                badge: Some(id.name().to_string()),
                text: format!(
                    "{} palette{}",
                    id.name(),
                    if id == current { " — current" } else { "" }
                ),
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
        self.message = format!("theme: {} palette", id.name());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    /// Restore the dark default on exit so neighbors in this test
    /// binary never observe another test's palette.
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            theme::set_current(ThemeId::Dark);
        }
    }

    fn editor() -> Editor {
        Editor::new_in(Buffer::from_text("x\n"), std::path::PathBuf::from("/proj"))
    }

    #[test]
    fn theme_command_switches_refuses_and_restores() {
        let _restore = Restore;
        let mut editor = editor();
        editor.theme_command("light");
        assert_eq!(theme::current_id(), ThemeId::Light);
        assert_eq!(editor.message, "theme: light palette");
        // Case-insensitive, like every other selector argument.
        editor.theme_command(" DARK ");
        assert_eq!(theme::current_id(), ThemeId::Dark);
        // A bogus name is a named refusal listing the real choices;
        // the live palette does not move.
        editor.theme_command("solarized");
        assert_eq!(theme::current_id(), ThemeId::Dark);
        assert_eq!(
            editor.message,
            "theme takes dark or light — not \"solarized\""
        );
    }

    #[test]
    fn bare_theme_opens_the_selector_and_a_row_applies() {
        let _restore = Restore;
        let mut editor = editor();
        editor.theme_command("light");
        editor.theme_command("");
        let glue = editor.picker.as_ref().expect("theme selector is open");
        assert_eq!(glue.picker.kind, strop_picker::Kind::Theme);
        let (ids, current_marks): (Vec<ThemeId>, Vec<bool>) = glue
            .picker
            .items
            .iter()
            .map(|item| {
                let Payload::ThemeChoice(id) = item.payload else {
                    panic!("theme rows carry theme payloads")
                };
                (id, item.text.ends_with(" — current"))
            })
            .unzip();
        assert_eq!(ids, vec![ThemeId::Dark, ThemeId::Light]);
        assert_eq!(current_marks, vec![false, true]);
        let _ = glue;
        editor.apply_theme(ThemeId::Dark);
        assert_eq!(theme::current_id(), ThemeId::Dark);
    }
}
