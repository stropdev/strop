//! Picker acceptance: what landing on a row DOES (open the file,
//! record the jump, surface errors as messages).

use strop_picker::Payload;

use super::super::Editor;

impl Editor {
    pub(crate) fn accept_picker(&mut self, payload: Payload) {
        match payload {
            Payload::File(rel) => {
                let path = self.cwd.join(&rel);
                match self.open_buffer(&path) {
                    Ok(()) => {}
                    Err(e) => self.message = format!("open {}: {e}", rel.display()),
                }
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
                // accepting a search/locations hit is a jump — same as gd
                self.push_jump();
                let full = self.cwd.join(&path);
                if let Err(e) = self.open_buffer(&full) {
                    self.message = format!("open {}: {e}", path.display());
                    return;
                }
                // same rule as LSP jumps: search hits outside the
                // workspace are reading, not editing
                let probe = self.cwd.join("x");
                let root = strop_lsp::registry::workspace_root(&probe, &self.cwd);
                if !full.starts_with(&root) && !self.buf().readonly {
                    self.buf_mut().readonly = true;
                    self.message = "readonly — outside workspace (:set noro to edit)".into();
                }
                let start = self.buf().line_start(line.saturating_sub(1));
                self.set_head(self.buf().clamp_boundary(start + col.saturating_sub(1)));
                self.clamp_cursor();
            }
        }
    }
}
