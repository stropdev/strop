//! `:tab-size` / `:indent-style` (0051 R08): per-buffer indent
//! overrides and the compact selector. Overrides win over detection
//! and config, survive reloads and config refreshes, and never rewrite
//! existing buffer bytes — they change rendering and new indentation.

use strop_picker::{IndentChoice, Item, Kind, Payload, Picker};

use super::document::IndentSource;
use super::Editor;
use crate::config::IndentStyle;

impl Editor {
    /// One effective setting for source editing, projected rows and carets.
    pub fn indentation_at(
        &self,
        document: strop_core::id::DocumentId,
        byte: usize,
    ) -> super::document::Indent {
        let source = self.indent_target_at(document, byte).unwrap_or(document);
        self.doc(source).indent
    }

    pub fn tab_width_for_location(&self, location: &strop_workspace::ResourceLocation) -> usize {
        let Ok(target) = crate::files::FileTarget::from_location(location) else {
            return self.config.tab_size;
        };
        self.docs
            .iter()
            .find(|(_, document)| document.matches_target(&target))
            .map_or(self.config.tab_size, |(_, document)| document.indent.width)
    }

    fn indent_target_at(
        &self,
        document: strop_core::id::DocumentId,
        byte: usize,
    ) -> Option<strop_core::id::DocumentId> {
        if let Some((source, _)) = self.source_position(document, byte) {
            return Some(source);
        }
        let view = self.docs.get(document)?;
        let collection = self.collections.get(&document)?;
        match collection.rows.get(view.buf.line_of(byte)) {
            Some(super::collections::CollectionRow::CardTop(index)) => collection
                .excerpts
                .get(*index)
                .map(|excerpt| excerpt.source),
            _ => None,
        }
    }

    fn indent_command_target(&mut self) -> Option<strop_core::id::DocumentId> {
        let target = self.indent_target_at(self.current(), self.head());
        if target.is_none() {
            self.message = "place the caret in a source excerpt to change indentation".into();
        }
        target
    }

    /// `:tab-size` — bare opens the selector; `N` sets a per-buffer
    /// width override (1–16, anything else refused visibly); `auto`
    /// clears the override and re-resolves from detection/config.
    pub(crate) fn tab_size_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_tab_size_picker();
            return;
        }
        let Some(document) = self.indent_command_target() else {
            return;
        };
        if arg.eq_ignore_ascii_case("auto") {
            self.set_width_override(document, None);
            return;
        }
        match arg.parse::<usize>() {
            Ok(width)
                if (crate::config::TAB_SIZE_MIN..=crate::config::TAB_SIZE_MAX).contains(&width) =>
            {
                self.set_width_override(document, Some(width));
            }
            _ => {
                self.message = format!(
                    "tab-size takes a width {}–{} or auto, not {arg:?}",
                    crate::config::TAB_SIZE_MIN,
                    crate::config::TAB_SIZE_MAX,
                );
            }
        }
    }

    /// `:indent-style spaces|tabs|auto` — the style side of the same
    /// override layer; bare opens the selector, which lists the styles.
    pub(crate) fn indent_style_command(&mut self, arg: &str) {
        if arg.trim().is_empty() {
            self.open_tab_size_picker();
            return;
        }
        let Some(document) = self.indent_command_target() else {
            return;
        };
        match arg.trim() {
            "spaces" => self.set_style_override(document, Some(IndentStyle::Spaces)),
            "tabs" => self.set_style_override(document, Some(IndentStyle::Tabs)),
            "auto" => self.set_style_override(document, None),
            other => {
                self.message = format!("indent-style takes spaces, tabs or auto, not {other:?}")
            }
        }
    }

    /// The modeline's indent segment: `Spaces:4` / `Tabs:4` (0051 R08).
    pub fn indent_label(&self) -> String {
        self.indentation_at(self.current(), self.head()).label()
    }

    /// The compact `:tab-size` selector (0051 R08): common widths, the
    /// style choices, Auto for both, and a pinned custom row that
    /// validates the typed number on accept. The current effective
    /// setting and its provenance are marked on the matching rows.
    pub(crate) fn open_tab_size_picker(&mut self) {
        let Some(document) = self.indent_command_target() else {
            return;
        };
        let indent = self.doc(document).indent;
        let current = |on: bool, source: IndentSource| {
            if on {
                format!(" — current ({})", source.label())
            } else {
                String::new()
            }
        };
        let mut items: Vec<Item> = [2usize, 3, 4, 8]
            .into_iter()
            .map(|width| Item {
                badge: Some(width.to_string()),
                text: format!(
                    "spaces per indent{}",
                    current(
                        indent.style == IndentStyle::Spaces && indent.width == width,
                        indent.width_source,
                    )
                ),
                payload: Payload::IndentChoice(IndentChoice::Width(width)),
            })
            .collect();
        items.push(Item {
            badge: None,
            text: format!(
                "auto width — detect/configure{}",
                current(
                    indent.width_source != IndentSource::Manual,
                    indent.width_source
                )
            ),
            payload: Payload::IndentChoice(IndentChoice::AutoWidth),
        });
        items.push(Item {
            badge: None,
            text: format!(
                "style: spaces{}",
                current(indent.style == IndentStyle::Spaces, indent.style_source)
            ),
            payload: Payload::IndentChoice(IndentChoice::Spaces),
        });
        items.push(Item {
            badge: None,
            text: format!(
                "style: tabs{}",
                current(indent.style == IndentStyle::Tabs, indent.style_source)
            ),
            payload: Payload::IndentChoice(IndentChoice::Tabs),
        });
        items.push(Item {
            badge: None,
            text: format!(
                "style: auto — detect/configure{}",
                current(
                    indent.style_source != IndentSource::Manual,
                    indent.style_source
                )
            ),
            payload: Payload::IndentChoice(IndentChoice::AutoStyle),
        });
        // Pinned tail: filtering never hides the custom row, and the
        // typed text becomes the width on accept (validated there).
        items.push(Item {
            badge: None,
            text: format!(
                "custom width… — type {}–{}, enter",
                crate::config::TAB_SIZE_MIN,
                crate::config::TAB_SIZE_MAX
            ),
            payload: Payload::IndentChoice(IndentChoice::CustomWidth),
        });
        let mut picker = Picker::new(Kind::TabSize, items, false);
        picker.pinned_tail = 1;
        let mut glue = super::picker::PickerGlue::diagnostics(picker);
        glue.indent_target = Some(document);
        self.set_picker(glue);
    }

    /// A selector row's choice. `draft` is the typed filter text — the
    /// custom row's width candidate, validated here (visible refusal).
    pub(crate) fn accept_indent_choice(
        &mut self,
        document: strop_core::id::DocumentId,
        choice: IndentChoice,
        draft: &str,
    ) {
        match choice {
            IndentChoice::Width(width) => self.set_width_override(document, Some(width)),
            IndentChoice::AutoWidth => self.set_width_override(document, None),
            IndentChoice::Spaces => self.set_style_override(document, Some(IndentStyle::Spaces)),
            IndentChoice::Tabs => self.set_style_override(document, Some(IndentStyle::Tabs)),
            IndentChoice::AutoStyle => self.set_style_override(document, None),
            IndentChoice::CustomWidth => match draft.parse::<usize>() {
                Ok(width)
                    if (crate::config::TAB_SIZE_MIN..=crate::config::TAB_SIZE_MAX)
                        .contains(&width) =>
                {
                    self.set_width_override(document, Some(width));
                }
                _ if draft.is_empty() => {
                    self.message = format!(
                        "type a width {}–{} first, then enter",
                        crate::config::TAB_SIZE_MIN,
                        crate::config::TAB_SIZE_MAX,
                    );
                }
                _ => {
                    self.message = format!(
                        "tab-size takes a width {}–{}, not {draft:?}",
                        crate::config::TAB_SIZE_MIN,
                        crate::config::TAB_SIZE_MAX,
                    );
                }
            },
        }
    }

    fn set_width_override(&mut self, document: strop_core::id::DocumentId, width: Option<usize>) {
        let Some(doc) = self.docs.get_mut(document) else {
            self.message = "indentation source was closed".into();
            return;
        };
        doc.indent_override.width = width;
        self.resolve_indent_for(document);
        self.message = self.indent_status(document);
    }

    fn set_style_override(
        &mut self,
        document: strop_core::id::DocumentId,
        style: Option<IndentStyle>,
    ) {
        let Some(doc) = self.docs.get_mut(document) else {
            self.message = "indentation source was closed".into();
            return;
        };
        doc.indent_override.style = style;
        self.resolve_indent_for(document);
        self.message = self.indent_status(document);
    }

    /// The post-command feedback: effective setting with per-side
    /// provenance — `Spaces:8 (width manual, style detected)`.
    fn indent_status(&self, document: strop_core::id::DocumentId) -> String {
        let indent = self.doc(document).indent;
        format!(
            "{}: {} (width {}, style {})",
            self.doc(document).label(&self.cwd),
            indent.label(),
            indent.width_source.label(),
            indent.style_source.label(),
        )
    }
}
