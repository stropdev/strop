//! normal/ex.rs — the ex command line: ranges, substitute, :w/:q family.

use crate::editor::Editor;

impl Editor {
    /// Ex-completion candidates for the pending prefix (name, doc).
    pub(crate) fn ex_candidates(&self) -> Vec<(&'static str, &'static str)> {
        let Some(prefix) = self.pending.strip_prefix(':') else {
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

    /// Tab on the ex line: cycle the completion candidates.
    pub(super) fn ex_tab_complete(&mut self) {
        let cands = self.ex_candidates();
        if cands.is_empty() {
            return;
        }
        let prefix = self.pending.strip_prefix(':').unwrap_or("");
        let next = cands
            .iter()
            .position(|(name, _)| *name == prefix)
            .map_or(cands[0].0, |i| cands[(i + 1) % cands.len()].0);
        self.pending = format!(":{next}");
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
                let text = self.buf().rope.byte_slice(s..e).to_string();
                self.registers.insert('\0', (text, true));
                let b = self.buf_mut();
                b.history.begin();
                b.delete(strop_core::Range::charwise(s, e));
                b.history.commit();
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
                let text = self.buf().rope.byte_slice(s..e).to_string();
                self.registers.insert('\0', (text, true));
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
        let text = self.buf().rope.byte_slice(s0..e0).to_string();
        let mut out = String::with_capacity(text.len());
        let mut hits = 0usize;
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if global {
                let n = line.matches(pat).count();
                hits += n;
                out.push_str(&line.replace(pat, repl));
            } else if let Some(p) = line.find(pat) {
                hits += 1;
                out.push_str(&line[..p]);
                out.push_str(repl);
                out.push_str(&line[p + pat.len()..]);
            } else {
                out.push_str(line);
            }
        }
        if hits == 0 {
            self.message = format!("pattern not found: {pat}");
            return;
        }
        {
            let b = self.buf_mut();
            b.history.begin();
            b.delete(strop_core::Range::charwise(s0, e0));
            b.insert(s0, &out);
            b.history.commit();
        }
        self.set_head(self.buf().clamp_boundary(s0));
        self.clamp_cursor();
        let end = (s0 + out.len()).min(self.buf().len_bytes());
        self.flash(strop_core::Range::charwise(s0, end));
        self.message = format!("{hits} substitution{}", if hits == 1 { "" } else { "s" });
    }

    pub(crate) fn run_ex(&mut self) {
        let cmdline = self
            .pending
            .trim_start_matches(':')
            .trim_end_matches('\r')
            .to_string();
        self.pending.clear();
        // vim ex ranges: [%, N, N.M, ., $, +/-offsets] prefix the
        // command. Bare :N is goto-line.
        let (range, rest) = self.parse_ex_range(&cmdline);
        if let Some((_, _)) = range {
            self.run_ranged_ex(range.unwrap(), rest);
            return;
        }
        let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline.as_str(), ""));
        match cmd {
            _ if cmdline.starts_with('!') => self.shell_run(&cmdline[1..]),
            "w" | "w!" => {
                // vim: :w {file} writes under a new name and adopts it
                let r = if arg.is_empty() {
                    self.buf_mut().save(cmd == "w!")
                } else {
                    self.buf_mut().save_as(arg)
                };
                match r {
                    Ok(()) => {
                        crate::session::save(self);
                        self.message = "written".into();
                    }
                    Err(e) => self.message = format!("write failed: {e}"),
                }
            }
            "wq" | "wq!" => {
                // a failed save keeps the buffer open and dirty — never
                // close into data loss (0014 wave 1)
                match self.buf_mut().save(cmd == "wq!") {
                    Ok(()) => {
                        crate::session::save(self);
                        // vim: :wq closes the WINDOW like :q — the shared
                        // document lives on in other panes (0015)
                        self.close_pane_or_buffer(false);
                    }
                    Err(e) => self.message = format!("write failed: {e}"),
                }
            }
            "set" => {
                // vim's option surface, narrowly: ro/noro only for now
                match arg {
                    "ro" | "readonly" => {
                        self.buf_mut().readonly = true;
                        self.message = "readonly".into();
                    }
                    "noro" | "noreadonly" => {
                        self.buf_mut().readonly = false;
                        self.message = "writable".into();
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
                } else if let Err(e) = self.open_buffer(arg) {
                    self.message = format!("view {arg}: {e}");
                } else {
                    self.buf_mut().readonly = true;
                }
            }
            "q" => {
                self.close_pane_or_buffer(false);
            }
            "q!" => {
                self.close_pane_or_buffer(true);
            }
            _ if cmdline.starts_with("s/") => {
                // :s without a range = the current line (vim)
                let line = self.buf().line_of(self.head());
                self.substitute_range(line, line, &cmdline[2..]);
            }
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
            "help" | "h" => self.open_help(),
            "e" | "e!" => {
                if arg.is_empty() {
                    self.message = ":e needs a path".into();
                } else if self.buf().dirty && cmd == "e" {
                    self.message = "unsaved changes — :e! to force".into();
                } else if let Err(e) = self.open_buffer(arg) {
                    self.message = format!("open {arg}: {e}");
                }
            }
            other => self.message = format!("unknown ex: :{other}"),
        }
    }
}
