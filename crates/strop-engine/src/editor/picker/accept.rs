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
            Payload::WorkspaceRoot(root) => match self.select_root(root) {
                Ok(_) => self.message = "project root selected".into(),
                Err(error) => self.message = error.to_string(),
            },
            Payload::RemoteDirectory(directory) => self.request_target(
                crate::files::FileTarget::Remote(directory.into()),
                super::super::io::OpenIntent::Browse,
            ),
            Payload::RemoteConnect => self.open_remote_address(),
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
            // Indentation choices retain a captured source owner and are
            // handled before closing the selector, never against current focus.
            Payload::IndentChoice(_) => {
                self.message = "indentation choice requires its original selector".into()
            }
            // Theme rows apply through their own selector, never the
            // generic accept path (same owner rule as indentation).
            Payload::ThemeChoice(_) => {
                self.message = "theme choice requires its original selector".into()
            }
            Payload::Action(_) => self.message = "action choice requires its original list".into(),
            // A project status row (0063 §2): Enter browses the project
            // root as a directory — never a symbol location jump.
            Payload::ProjectStatus(root) => {
                self.request_open(root, super::super::io::OpenIntent::Browse);
            }
            Payload::File(_) => {
                self.message = "file choice requires its captured project scope".into()
            }
            Payload::Buffer(i) => {
                if self.docs.get(i).is_some() {
                    self.push_jump();
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
                    self.lsp_jump_from_picker(location, line, col, context);
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
