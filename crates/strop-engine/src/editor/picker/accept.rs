//! Picker acceptance: open the selected file and preserve coordinate domains.
use super::super::Editor;
use strop_picker::Payload;

impl Editor {
    pub(crate) fn accept_picker(
        &mut self,
        payload: Payload,
        context: Option<strop_lsp::ReplyContext>,
    ) {
        match payload {
            Payload::RemoteDirectory(directory) => self.request_target(
                crate::files::FileTarget::Remote(directory.into()),
                super::super::io::OpenIntent::Browse,
            ),
            Payload::RemoteConnect => self.open_remote_address(),
            // R03: the toggle flips the session default and the row
            // re-renders with the new value
            Payload::SearchOption(setting) => {
                match setting {
                    strop_picker::SearchSetting::Hidden => {
                        self.config.search_show_hidden = !self.config.search_show_hidden;
                    }
                    strop_picker::SearchSetting::RespectIgnore => {
                        self.config.search_respect_ignore = !self.config.search_respect_ignore;
                    }
                }
                // SearchOption payloads only exist in that picker:
                // reopen it with the fresh values
                self.open_search_options();
            }
            // A jumplist menu entry: record the present position first,
            // so ctrl-o after the picker jump returns here (0047 §2).
            Payload::Jump { document, offset } => {
                if self.docs.get(document).is_some() {
                    self.push_jump();
                    // accepting a menu entry is a new landing (0051 §7):
                    // deliberate placement, not a ctrl-o view restore
                    self.jump_land(document, offset);
                }
            }
            Payload::CodeAction(index) => self.accept_code_action(index),
            Payload::Container(id) => self.attach_container(id),
            // Indentation choices retain a captured source owner and are
            // handled before closing the selector, never against current focus.
            Payload::IndentChoice(_) => {
                self.message = "indentation choice requires its original selector".into()
            }
            Payload::FilesystemAction(_) => {
                self.message = "filesystem choice requires its original selector".into()
            }
            Payload::File(rel) => {
                self.request_open(
                    rel,
                    super::super::io::OpenIntent::Switch { readonly: false },
                );
            }
            Payload::Buffer(i) => {
                if self.docs.get(i).is_some() {
                    self.switch_to(i);
                    self.set_head(0);
                    self.view_mut().view_top = 0;
                }
            }
            Payload::Grep {
                location,
                line,
                col,
                ..
            } => {
                if let strop_workspace::Filesystem::Remote(endpoint) = &location.filesystem {
                    self.lsp_open_remote_hit(endpoint, &location.path, line, col, context);
                    return;
                }
                if let Some(context) = context {
                    self.lsp_jump_from_picker(location.path, line, col, context);
                    return;
                }
                // Grep/symbol hits are jumps in vim's sense (quickfix
                // jumps enter the jumplist): record first, so ctrl-o
                // returns to where the picker was accepted (0047 §1).
                self.push_jump();
                let target = match crate::files::FileTarget::from_location(&location) {
                    Ok(target) => target,
                    Err(error) => {
                        self.message = error.to_string();
                        return;
                    }
                };
                self.request_target(
                    target,
                    super::super::io::OpenIntent::Grep {
                        line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                        column: strop_core::id::ByteColumn::new(col.saturating_sub(1)),
                    },
                );
            }
            // A remote hit routes through the endpoint's file identity
            // (0036): the analogous local path is never opened.
            Payload::Remote {
                endpoint,
                path,
                line,
                col,
            } => {
                self.lsp_open_remote_hit(&endpoint, &path, line, col, context);
            }
        }
    }
}
