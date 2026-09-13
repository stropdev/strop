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

    /// Parse a leading ex range: `%`, `.`, `$`, `N`, `N,M`, with
    /// +/- offsets. Returns 0-indexed inclusive line bounds + the
    /// remaining command text, or None when no range leads.
    fn parse_ex_range<'a>(&self, cmdline: &'a str) -> (Option<(usize, usize)>, &'a str) {
        let buf = self.buf();
        let last = buf.last_content_line();
        let cur = buf.line_of(self.head());
        let addr = |tok: &str| -> Option<(usize, usize)> {
            // one address + the bytes it consumed
            match tok.as_bytes().first()? {
                b'%' => Some((0, 1)),
                b'.' => Some((cur, 1)),
                b'$' => Some((last, 1)),
                b if b.is_ascii_digit() => {
                    let n: usize = tok
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .ok()?;
                    Some((n.saturating_sub(1).min(last), n.to_string().len()))
                }
                _ => None,
            }
        };
        let (mut first, mut used) = match addr(cmdline) {
            Some(v) => v,
            None => return (None, cmdline),
        };
        let mut second = None;
        if cmdline.as_bytes().get(used) == Some(&b',') {
            match addr(&cmdline[used + 1..]) {
                Some((l2, u2)) => {
                    second = Some(l2);
                    used += 1 + u2;
                }
                None => return (None, cmdline),
            }
        }
        // +/- offsets trail an address (:+3, :-2, :.-1,$-1)
        let whole = cmdline[..used].to_string();
        let mut tail = &cmdline[used..];
        let apply_off = |line: usize, tail: &str| -> (usize, usize) {
            let b = tail.as_bytes();
            let mut i = 0;
            let mut line = line;
            while i < tail.len() && (b[i] == b'+' || b[i] == b'-') {
                let neg = b[i] == b'-';
                i += 1;
                let digits: String = tail[i..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                let n: usize = if digits.is_empty() {
                    1
                } else {
                    digits.parse().unwrap_or(1)
                };
                i += digits.len();
                line = if neg {
                    line.saturating_sub(n)
                } else {
                    (line + n).min(last)
                };
            }
            (line, i)
        };
        let (f, fu) = apply_off(first, tail);
        first = f;
        tail = &tail[fu..];
        let mut last_line = second.unwrap_or(first);
        if second.is_some() {
            let (l, lu) = apply_off(last_line, tail);
            last_line = l;
            tail = &tail[lu..];
        }
        if whole == "%" {
            return (Some((0, last)), tail);
        }
        if first > last_line {
            return (None, cmdline); // backwards range: vim errors
        }
        (Some((first.min(last), last_line.min(last))), tail)
    }

    /// Ranged commands: `:N` alone jumps; `d`/`y` delete/yank the
    /// lines; `s/a/b/[g]` substitutes (LITERAL pattern — vim's regex
    /// substitute is a documented deviation until 0016's grammar work).
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
            _ if rest.starts_with("s/") => self.substitute_range(lo, hi, &rest[2..]),
            _ => self.message = format!("unsupported ranged command: {rest}"),
        }
    }

    /// `:[range]s/pat/repl/[g]` — literal pattern, vim's flag letter g.
    fn substitute_range(&mut self, lo: usize, hi: usize, spec: &str) {
        let parts: Vec<&str> = spec.split('/').collect();
        if parts.len() < 2 {
            self.message = ":s needs /pat/repl/".into();
            return;
        }
        let (pat, repl) = (parts[0], parts[1]);
        let global = parts.get(2).is_some_and(|f| f.contains('g'));
        if pat.is_empty() {
            self.message = "empty pattern".into();
            return;
        }
        let s0 = self.buf().line_start(lo);
        let e0 = self.buf().line_end(hi);
        let text = self.buf().text().byte_slice(s0..e0).to_string();
        let mut edits = Vec::new();
        let mut offset = s0;
        for line in text.split_inclusive('\n') {
            for (start, _) in line.match_indices(pat) {
                edits.push(strop_core::Replacement::new(
                    strop_core::Range::charwise(offset + start, offset + start + pat.len()),
                    repl,
                ));
                if !global {
                    break;
                }
            }
            offset += line.len();
        }
        let hits = edits.len();
        if hits == 0 {
            self.message = format!("pattern not found: {pat}");
            return;
        }
        if let Err(error) = self.apply(
            self.current(),
            self.buf().revision(),
            crate::editor::transact::ChangeSet {
                edits,
                undo_open: false,
            },
        ) {
            self.message = format!("substitution failed: {error}");
            return;
        }
        self.set_head(self.buf().clamp_boundary(s0));
        self.clamp_cursor();
        let end = e0
            .saturating_add_signed((repl.len() as isize - pat.len() as isize) * hits as isize)
            .min(self.buf().len_bytes());
        self.flash(strop_core::Range::charwise(s0, end));
        self.message = format!("{hits} substitution{}", if hits == 1 { "" } else { "s" });
    }

    pub(crate) fn run_ex(&mut self, cmdline: &str) {
        // vim ex ranges: [%, N, N.M, ., $, +/-offsets] prefix the
        // command. Bare :N is goto-line.
        let (range, rest) = self.parse_ex_range(cmdline);
        if let Some(range) = range {
            self.run_ranged_ex(range, rest);
            return;
        }
        let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline, ""));
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
            "w" | "w!" if self.collections.contains_key(&self.current()) => {
                self.collection_save((!arg.is_empty()).then(|| arg.into()), cmd == "w!", false);
            }
            "w" | "w!" => {
                // vim: readonly buffers refuse plain :w (surfaces, :view);
                // :w! forces through the mutation boundary's rule
                // Container files have no write path (0037 DC1b): refuse
                // both forms — never a local-path fallback, never w!.
                if matches!(
                    self.cur().source,
                    crate::editor::document::DocumentSource::Container { .. }
                ) {
                    self.message =
                        "container files are read-only (0037 DC1b); no in-container save".into();
                    return;
                }
                if self.buf().readonly && cmd != "w!" && self.remote_file().is_none() {
                    let name = self.buf().name.as_deref().unwrap_or("readonly buffer");
                    self.message = format!("{name}: readonly — :w! to force");
                    return;
                }
                self.request_save((!arg.is_empty()).then(|| arg.into()), cmd == "w!", false);
            }
            "wq" | "wq!" if self.collections.contains_key(&self.current()) => {
                self.collection_save(None, cmd == "wq!", true);
            }
            "wq" | "wq!" => {
                if matches!(
                    self.cur().source,
                    crate::editor::document::DocumentSource::Container { .. }
                ) {
                    self.message =
                        "container files are read-only (0037 DC1b); no in-container save".into();
                    return;
                }
                self.request_save((!arg.is_empty()).then(|| arg.into()), cmd == "wq!", true);
            }
            "set" => {
                // vim's option surface, narrowly: ro/noro only for now
                match arg {
                    "ro" | "readonly" => {
                        self.buf_mut().readonly = true;
                        self.message = "readonly".into();
                    }
                    "noro" | "noreadonly" => {
                        if self.directory().is_some_and(|source| {
                            source.draft.as_ref().is_none_or(|draft| !draft.editable())
                        }) {
                            self.message = "Directory listings are read-only; use :fs edit for filename drafts".into();
                            return;
                        }
                        if matches!(
                            self.cur().source,
                            crate::editor::DocumentSource::Container { .. }
                        ) {
                            self.message = "container files are read-only by policy".into();
                            return;
                        }
                        if self.remote_file().is_some() && !self.remote_edit_authorized() {
                            self.message =
                                "remote file is read-only; use :remote edit first".into();
                        } else {
                            self.buf_mut().readonly = false;
                            self.message = "writable".into();
                        }
                    }
                    _ => self.message = format!("unknown option: {arg}"),
                }
            }
            "view" => {
                // vim view: edit readonly — no arg marks the current
                // buffer readonly
                if arg.is_empty() {
                    self.buf_mut().readonly = true;
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
            "qa" | "qall" => self.quit_all(false),
            "qa!" | "qall!" => self.quit_all(true),
            _ if cmdline.starts_with("s/") => {
                // :s without a range = the current line (vim)
                let line = self.buf().line_of(self.head());
                self.substitute_range(line, line, &cmdline[2..]);
            }
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
            "indent-style" => self.indent_style_command(arg),
            "apply-change" => self.review_apply_pub(),
            "select-next" => self.occurrence_next_pub(),
            "select-all" => self.occurrence_all_pub(),
            "select-skip" => self.occurrence_skip_pub(),
            "select-pop" => self.occurrence_pop_pub(),
            "cancel-change" => self.review_cancel_pub(),
            "collection" if arg == "source" => self.collection_open_source(),
            "collection" if arg == "expand" => self.collection_context_step(true),
            "collection" if arg == "contract" => self.collection_context_step(false),
            "collection" => {
                self.message = ":collection source — open the full source at the caret".into()
            }
            "symbols" => self.lsp_document_symbols_pub(),
            "explain" => self.open_explain(),
            "containers" => self.request_containers(),
            "format" => self.lsp_format(),
            "rename" => {
                if arg.is_empty() {
                    self.message = ":rename needs a new name".into();
                } else {
                    self.lsp_rename(arg);
                }
            }
            "undo-change" => self.undo_last_change(),
            "save-change" => self.save_changed_files_pub(),
            "e" | "e!" => {
                if self.buf().dirty && cmd == "e" {
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
