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
            // A jumplist menu entry: record the present position first,
            // so ctrl-o after the picker jump returns here (0047 §2).
            Payload::Jump { document, offset } => {
                if self.docs.get(document).is_some() {
                    self.push_jump();
                    self.jump_to((document, offset));
                }
            }
            Payload::CodeAction(index) => self.accept_code_action(index),
            Payload::Container(id) => self.attach_container(id),
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
                path, line, col, ..
            } => {
                if let Some(context) = context {
                    self.lsp_jump_from_picker(path, line, col, context);
                    return;
                }
                // Grep/symbol hits are jumps in vim's sense (quickfix
                // jumps enter the jumplist): record first, so ctrl-o
                // returns to where the picker was accepted (0047 §1).
                self.push_jump();
                self.request_open(
                    path,
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
