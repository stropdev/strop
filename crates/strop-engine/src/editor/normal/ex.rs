//! normal/ex.rs — the ex command line: ranges, substitute, :w/:q family.

use crate::editor::Editor;

use crate::editor::Register;

impl Editor {
    /// Ex-completion candidates for the pending prefix (name, doc).
    pub fn ex_candidates(&self) -> Vec<(&'static str, &'static str)> {
        let Some(prefix) = self.pending.text().strip_prefix(':') else {
            return Vec::new();
        };
        if prefix.contains(' ') {
            return Vec::new();
        }
        super::EX_COMMANDS
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .copied()
            .collect()
    }

    /// Tab on the ex line: cycle the completion candidates — command
    /// names for a bare prefix; remote hosts/paths for a file
    /// command's `ssh://` operand (editor/remote_completion.rs).
    pub(super) fn ex_tab_complete(&mut self) {
        if self.remote_completion_tab() {
            return;
        }
        let cands = self.ex_candidates();
        if cands.is_empty() {
            return;
        }
        let prefix = self.pending.text().strip_prefix(':').unwrap_or("");
        let next = cands
            .iter()
            .position(|(name, _)| *name == prefix)
            .map_or(cands[0].0, |i| cands[(i + 1) % cands.len()].0);
        self.feed_pending_event(crate::editor::pending::PendingEvent::CompleteEx(
            next.to_owned(),
        ));
    }

    /// Parse a leading ex range through the same pure parser used by
    /// substitute composition and execution.
    fn parse_ex_range<'a>(
        &self,
        cmdline: &'a str,
    ) -> Result<super::substitute::ExRangeParse<'a>, String> {
        super::substitute::parse_ex_range(self.buf(), self.head(), cmdline)
    }

    /// Ranged commands: `:N` alone jumps; `d`/`y` delete/yank the
    /// lines. Substitute is dispatched before this command-family match.
    fn run_ranged_ex(&mut self, range: (usize, usize), rest: &str) {
        let (lo, hi) = range;
        if let Some(argument) = rest.strip_prefix("fs ") {
            self.run_filesystem_ex(argument, Some(range));
            return;
        }
        if rest.is_empty() {
            // :N — goto line
            let s = self.buf().line_start(lo);
            self.set_head(s);
            self.clamp_cursor();
            self.scroll_to_cursor(self.view_rows());
            return;
        }
        match rest {
            "d" | "d!" => {
                let s = self.buf().line_start(lo);
                let e = if hi + 1 < self.buf().len_lines() {
                    self.buf().line_start(hi + 1)
                } else {
                    self.buf().len_bytes()
                };
                let text = self.buf().text().byte_slice(s..e).to_string();
                let mut register = Register::linewise(text);
                register.file_provenance = self.capture_filename_register(
                    [(strop_core::Range::charwise(s, e), true)],
                    true,
                    false,
                );
                self.set_register(None, register);
                self.tx_begin();
                self.filename_delete_hint(strop_core::Range::charwise(s, e), true);
                self.buf_mut().delete(strop_core::Range::charwise(s, e));
                self.clear_filename_hint();
                self.tx_commit();
                self.set_head(self.buf().clamp_boundary(s));
                self.clamp_cursor();
                self.message = format!("{} lines deleted", hi - lo + 1);
            }
            "y" => {
                let s = self.buf().line_start(lo);
                let e = if hi + 1 < self.buf().len_lines() {
                    self.buf().line_start(hi + 1)
                } else {
                    self.buf().len_bytes()
                };
                let text = self.buf().text().byte_slice(s..e).to_string();
                let mut register = Register::linewise(text);
                register.file_provenance = self.capture_filename_register(
                    [(strop_core::Range::charwise(s, e), true)],
                    false,
                    false,
                );
                self.set_register(None, register);
                self.message = format!("{} lines yanked", hi - lo + 1);
            }
            _ => self.message = format!("unsupported ranged command: {rest}"),
        }
    }

    pub(crate) fn run_ex(&mut self, cmdline: &str) {
        // vim ex ranges: [%, N, N.M, ., $, +/-offsets] prefix the
        // command. Bare :N is goto-line.
        let (range, rest) = match self.parse_ex_range(cmdline) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if super::substitute::is_substitute(rest) {
            let line = self.buf().line_of(self.head());
            self.run_substitute(range.unwrap_or((line, line)), rest);
            return;
        }
        if let Some(range) = range {
            self.run_ranged_ex(range, rest);
            return;
        }
        let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline, ""));
        match cmd {
            "terminal" | "term" => {
                self.launch_terminal(arg, false);
                return;
            }
            "terminal-local" => {
                self.launch_terminal(arg, true);
                return;
            }
            "r" | "read" => {
                self.read_command(arg);
                return;
            }
            _ => {}
        }
        if cmd == "fs" {
            self.run_filesystem_ex(arg, None);
            return;
        }
        if matches!(cmd, "browse" | "filter") {
            let result = if cmd == "browse" {
                self.browse_directory(arg.trim())
            } else {
                self.filter_directory(arg.to_owned())
            };
            if let Err(error) = result {
                self.message = error;
            }
            return;
        }
        if self.run_remote_ex(cmd, arg) {
            return;
        }
        match cmd {
            _ if cmdline.starts_with('!') => self.shell_run(&cmdline[1..]),
            "w" | "w!" | "wq" | "wq!" if self.collections.contains_key(&self.current()) => {
                self.collection_save(cmd.starts_with("wq"));
            }
            "w" | "w!" => {
                // vim: readonly buffers refuse plain :w (surfaces, :view);
                // :w! forces through the mutation boundary's rule.
                if self.buf().readonly && cmd != "w!" && self.remote_file().is_none() {
                    let name = self.buf().name.as_deref().unwrap_or("readonly buffer");
                    self.message = format!("{name}: readonly — :w! to force");
                    return;
                }
                self.request_save((!arg.is_empty()).then(|| arg.into()), cmd == "w!", false);
            }
            "wq" | "wq!" => {
                self.request_save((!arg.is_empty()).then(|| arg.into()), cmd == "wq!", true);
            }
            "set" => {
                // vim's option surface, narrowly: ro/noro only for now
                match arg {
                    "ro" | "readonly" => {
                        self.buf_mut()
                            .set_readonly(strop_core::ReadonlyReason::Command);
                        self.message = "readonly".into();
                    }
                    "noro" | "noreadonly" => {
                        if self.terminal_document(self.current()).is_some() {
                            self.message =
                                "terminal buffer is read-only — i enters TERMINAL mode".into();
                            return;
                        }
                        if self.directory().is_some_and(|source| {
                            source.draft.as_ref().is_none_or(|draft| !draft.editable())
                        }) {
                            self.message = "Directory listings are read-only; use :fs edit for filename drafts".into();
                            return;
                        }
                        self.buf_mut().clear_readonly();
                        self.message = "writable".into();
                    }
                    _ => self.message = format!("unknown option: {arg}"),
                }
            }
            "view" => {
                // vim view: edit readonly — no arg marks the current
                // buffer readonly
                if arg.is_empty() {
                    self.buf_mut()
                        .set_readonly(strop_core::ReadonlyReason::Command);
                    self.message = "readonly".into();
                } else {
                    self.request_user_open(
                        arg,
                        super::super::io::OpenIntent::Switch { readonly: true },
                    );
                }
            }
            "q" => {
                self.close_pane_or_buffer(false);
            }
            "q!" => {
                self.close_pane_or_buffer(true);
            }
            "bd" | "bdelete" => self.delete_buffer(false),
            "bd!" | "bdelete!" => self.delete_buffer(true),
            "qa" | "qall" => self.quit_all(false),
            "qa!" | "qall!" => self.quit_all(true),
            "trust" => self.request_trust(),
            "noh" => {
                // nohlsearch: the persistent highlight drops (0001 §5.8)
                self.last_search = None;
            }
            _ if cmdline.bytes().all(|b| b.is_ascii_digit()) && !cmdline.is_empty() => {
                // :30 jumps to line 30 (vim); past EOF clamps to the last
                // content line, never the phantom past a trailing newline
                let n: usize = cmdline.parse().unwrap_or(1);
                let mut last = self.buf().len_lines().saturating_sub(1);
                if self.buf().line_start(last) >= self.buf().len_bytes() {
                    last = last.saturating_sub(1);
                }
                self.push_jump(); // :N is a jump — record before moving
                self.set_head(self.buf().line_start(n.saturating_sub(1).min(last)));
                self.clamp_cursor();
            }
            "vs" | "vsplit" => self.split(true, if arg.is_empty() { None } else { Some(arg) }),
            "sp" | "split" => self.split(false, if arg.is_empty() { None } else { Some(arg) }),
            "help" | "h" => self.open_help_topic(arg),
            "jumps" => self.open_jumps_picker(),
            "search-options" => self.open_search_options(),
            "tab-size" => self.tab_size_command(arg),
            "theme" => self.theme_command(arg),
            "indent-style" => self.indent_style_command(arg),
            "copen" | "cw" | "cwindow" => self.quickfix_open(),
            "cnext" | "cn" => self.quickfix_step(1),
            "cprev" | "cp" | "cprevious" | "cN" => self.quickfix_step(-1),
            "cc" => self.quickfix_goto(arg),
            "cdo" => self.quickfix_do(arg, false),
            "cfdo" => self.quickfix_do(arg, true),
            "select-next" => self.occurrence_next_pub(),
            "select-all" => self.occurrence_all_pub(),
            "select-skip" => self.occurrence_skip_pub(),
            "select-pop" => self.occurrence_pop_pub(),
            "collection" if arg == "source" => self.collection_open_source(),
            "collection" if arg == "expand" => self.collection_context_step(true),
            "collection" if arg == "contract" => self.collection_context_step(false),
            "collection" => {
                self.message = ":collection source — open the full source at the caret".into()
            }
            "symbols" => self.lsp_document_symbols_pub(),
            "explain" => self.open_explain(),
            "messages" => self.open_messages(),
            "recover" => self.run_recovery_ex(arg),
            "format" => self.lsp_format(),
            "rename" => {
                if arg.is_empty() {
                    self.message = ":rename needs a new name".into();
                } else {
                    self.lsp_rename(arg);
                }
            }
            "e" | "e!" => {
                if self.buf().is_dirty() && cmd == "e" {
                    self.message = "unsaved changes — :e! to force".into();
                } else if arg.is_empty() {
                    if !self.refresh_current_resource() {
                        self.message = "no file or directory to reload".into();
                    }
                } else {
                    self.request_user_open(
                        arg,
                        super::super::io::OpenIntent::Switch { readonly: false },
                    );
                }
            }
            other => self.message = format!("unknown ex: :{other}"),
        }
    }
}
