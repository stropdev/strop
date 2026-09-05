//! Registers and paste (vim's " machinery) plus the system clipboard
//! (`+` via OSC52/wl-paste, helix's playbook). Reads run on workers;
//! results land on the drain — never a subprocess on the input path.

use super::Editor;

impl Editor {
    pub(crate) fn register(&self, name: Option<char>) -> &(String, bool) {
        static EMPTY: (String, bool) = (String::new(), false);
        self.registers.get(&name.unwrap_or('"')).unwrap_or(&EMPTY)
    }

    pub(crate) fn set_register(&mut self, name: Option<char>, text: String, linewise: bool) {
        // the `+` register is the system clipboard: yank/delete into it
        // stages an OSC52 payload for the TUI to emit
        if name == Some('+') {
            self.osc52 = Some(text.clone());
        }
        self.registers.insert(name.unwrap_or('"'), (text, linewise));
    }

    /// Counted paste (vim `2p`): the register lands `count` times at
    /// one position, one undo unit.
    pub(crate) fn paste_n(&mut self, count: usize, before: bool) {
        let (text, linewise) = self.register(None).clone();
        if text.is_empty() {
            return;
        }
        self.paste_text(text.repeat(count), linewise, before);
    }

    pub(crate) fn paste(&mut self, name: Option<char>, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        // `"+p`: the system clipboard is read by a provider job — never
        // a subprocess on the input path (0001 §3)
        if name == Some('+') {
            self.clipboard_paste(before);
            return;
        }
        let (text, linewise) = self.register(name).clone();
        if text.is_empty() {
            return;
        }
        self.paste_text(text, linewise, before);
    }

    /// `Space p` / `"+p`: spawn a clipboard read; the result lands in
    /// drain_clipboard on a later tick.
    pub(crate) fn clipboard_paste(&mut self, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        if self.clip_paste_pending.is_some() {
            return; // one read in flight
        }
        self.clip_paste_pending = Some(before);
        let tx = self.clip_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(read_system_clipboard());
        });
    }

    /// Collect clipboard reads (event-loop tick + headless settle).
    pub fn drain_clipboard(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        while let Ok(result) = self.clip_rx.try_recv() {
            let Some(before) = self.clip_paste_pending.take() else {
                continue;
            };
            match result {
                Some(text) if !text.is_empty() => {
                    let linewise = text.len() > 1 && text.ends_with('\n');
                    self.paste_text(text, linewise, before);
                }
                _ => {
                    self.message =
                        "clipboard: empty or no provider (wl-paste/xclip/xsel/pbpaste)".into()
                }
            }
        }
    }

    /// Insertion point + landing spot for one cursor's paste.
    fn paste_points(
        &self,
        cursor: usize,
        text_len: usize,
        linewise: bool,
        before: bool,
    ) -> (usize, usize) {
        if linewise {
            let line = self.buf().line_of(cursor);
            let at = if before {
                self.buf().line_start(line)
            } else {
                self.buf().line_start(line + 1)
            };
            (
                at.min(self.buf().len_bytes()),
                at.min(self.buf().len_bytes()),
            )
        } else {
            let at = if before {
                cursor
            } else {
                (cursor + 1).min(self.buf().len_bytes())
            };
            // vim: the cursor lands on the LAST pasted char, both p and P
            let land = at + text_len.saturating_sub(1);
            (at, land)
        }
    }

    fn paste_text(&mut self, text: String, linewise: bool, before: bool) {
        // nvim rule: every command is one undo unit — a lone paste must
        // commit its own revision (it used to ride the *next* command's)
        self.tx_begin();
        let cursors = self.all_cursors();
        if cursors.len() == 1 {
            let (at, land) = self.paste_points(self.head(), text.len(), linewise, before);
            self.buf_mut().insert(at, &text);
            self.set_head(land);
            self.clamp_cursor();
            self.tx_commit();
            return;
        }
        // multicursor paste (0013 §3): same text at every cursor,
        // bottom-up so insertion points stay valid mid-batch
        let primary = self.head();
        let mut jobs: Vec<(usize, usize, bool)> = cursors
            .into_iter()
            .map(|c| {
                let (at, land) = self.paste_points(c, text.len(), linewise, before);
                (at, land, c == primary)
            })
            .collect();
        jobs.sort_by_key(|j| j.0);
        jobs.dedup_by_key(|j| j.0); // stacked cursors paste once
                                    // each landing shifts by what lower insertions already added
        let mut shift = 0usize;
        for j in &mut jobs {
            j.1 += shift;
            shift += text.len();
        }
        for (at, _, _) in jobs.iter().rev() {
            self.buf_mut().insert(*at, &text);
        }
        self.sels_mut()
            .set_extras(jobs.iter().filter(|j| !j.2).map(|j| j.1));
        self.set_head(jobs.iter().find(|j| j.2).map(|j| j.1).unwrap_or(primary));
        self.normalize_cursors();
        self.clamp_cursor();
        self.tx_commit();
    }
}

/// Read the system clipboard via the first working provider (helix's
/// playbook: wl-paste, xclip, xsel, pbpaste). Runs on a worker thread.
fn read_system_clipboard() -> Option<String> {
    let providers: [(&str, &[&str]); 4] = [
        ("wl-paste", &[]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
        ("pbpaste", &[]),
    ];
    for (cmd, args) in providers {
        let Ok(out) = std::process::Command::new(cmd).args(args).output() else {
            continue; // not installed
        };
        if out.status.success() {
            return String::from_utf8(out.stdout).ok();
        }
    }
    None
}

impl Editor {
    /// Table shims (0008 stage 2): `Space y` arms the `+` register for
    /// the next yank; `Space p/P` paste from the system clipboard.
    pub(crate) fn clipboard_yank_pub(&mut self) {
        self.pending = "\"+y".into();
    }
    pub(crate) fn clipboard_paste_pub(&mut self, before: bool) {
        self.clipboard_paste(before);
    }
    /// Bracketed paste (0017): one undo unit, no key interpretation —
    /// the payload is text, not keystrokes. In normal mode it behaves
    /// like p; a trailing newline pastes linewise (vim's paste plugin
    /// convention).
    pub fn paste_bracketed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.mode == super::Mode::Insert {
            let pos = self.head();
            self.tx_begin();
            self.buf_mut().insert(pos, text);
            self.tx_commit();
            self.set_head(pos + text.len());
            self.clamp_cursor();
        } else {
            let linewise = text.ends_with('\n');
            self.paste_text(text.to_string(), linewise, false);
        }
    }
}
