//! What the editor does with the command line: open it, feed it keys,
//! run it, cancel it, and keep incsearch on the pattern being typed.

use strop_core::frontend_input::Key;
use strop_core::Range;
use strop_grammar::{self as grammar, Command};

use super::{keys::Action, Context, Line, Origin};
use crate::editor::ex::complete::{self, Step};
use crate::editor::{input::ParserState, Editor, Mode};

impl Editor {
    fn cmdline_origin(&self) -> Origin {
        Origin {
            pane_index: self.active_pane,
            pane: self.view().clone(),
            revision: self.buf().revision(),
        }
    }

    /// Open the `:` line with `body` already typed (`!{motion}`).
    pub(crate) fn open_ex_line(&mut self, body: &str) {
        self.cancel_cmdline();
        let origin = self.cmdline_origin();
        self.cmdline
            .open(Line::with_body(Context::Ex(origin), body));
        self.cmdline_parse();
    }

    /// A line opened (`: / ? |`) with the typed entry state that
    /// survived the crossing — counts, register, operator.
    pub(crate) fn begin_text_line(&mut self, sigil: char, state: ParserState) {
        self.cancel_cmdline();
        // Vim: `:` ends Visual mode. The selection stays behind as the
        // marks `'<` and `'>` (`feed_visual` recorded it on this key), and the
        // line opens with that range typed.
        let visual = matches!(
            self.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        );
        if sigil == ':' && visual {
            let selection = self.sels().primary();
            self.mode = Mode::Normal;
            // Vim leaves the cursor at the selection's start.
            let start = selection.anchor.min(selection.head);
            self.sels_mut().stretch_primary(start, start);
            let origin = self.cmdline_origin();
            self.cmdline
                .open(Line::with_body(Context::Ex(origin), "'<,'>"));
            self.cmdline_parse();
            return;
        }
        let origin = self.cmdline_origin();
        let context = match sigil {
            ':' => Context::Ex(origin),
            '/' | '?' => Context::Search {
                origin,
                state,
                backward: sigil == '?',
            },
            '|' => {
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    return;
                }
                let range = if visual {
                    let Some(range) = self.visual_range() else {
                        return;
                    };
                    range
                } else {
                    let line = self.buf().line_of(self.head());
                    Range::charwise(
                        self.buf().line_start(line),
                        self.buf().line_start(line + 1).min(self.buf().len_bytes()),
                    )
                };
                Context::Pipe {
                    origin,
                    range,
                    visual,
                }
            }
            _ => unreachable!("Walker only emits text-line sigils"),
        };
        self.cmdline.open(Line::new(context));
    }

    /// The saved origin still describes this editor: same pane slot,
    /// same document incarnation, unchanged revision. Service results
    /// and edits invalidate it; delivery-time checks are the backstop.
    pub(crate) fn cmdline_origin_valid(&self, origin: &Origin) -> bool {
        self.active_pane == origin.pane_index
            && self
                .panes
                .get(origin.pane_index)
                .is_some_and(|p| p.doc == origin.pane.doc)
            && self
                .docs
                .get(origin.pane.doc)
                .is_some_and(|d| d.buf.revision() == origin.revision)
    }

    fn restore_cmdline_origin(&mut self, origin: &Origin) -> bool {
        if !self.cmdline_origin_valid(origin) {
            return false;
        }
        // Never clamp/normalize: those operations would change saved
        // anchors, duplicate cursors or a deliberately parked viewport.
        self.panes[origin.pane_index] = origin.pane.clone();
        true
    }

    /// Close the open line (if any), restoring its origin. Called before
    /// anything replaces the pane/document/revision the line was opened
    /// against — never on rejected service results.
    pub(crate) fn cancel_cmdline(&mut self) {
        if let Some(line) = self.cmdline.close() {
            self.resolution.cancel_preview();
            self.restore_cmdline_origin(line.origin());
        }
    }

    /// A line whose origin went stale (the document or pane changed
    /// under it) closes before it takes another key.
    fn cmdline_live(&mut self) -> bool {
        let Some(line) = self.cmdline.line() else {
            return false;
        };
        if self.cmdline_origin_valid(line.origin()) {
            return true;
        }
        self.cancel_cmdline();
        self.message = "input cancelled: document or pane changed".into();
        false
    }

    /// One key for the open line: the key table, or a typed character.
    pub(crate) fn feed_cmdline(&mut self, key: Key) {
        if !self.cmdline_live() {
            return;
        }
        let Some(line) = self.cmdline.line_mut() else {
            return;
        };
        if line.take_register() {
            match key {
                Key::Char(name) => {
                    self.paste_from(
                        Some(name),
                        crate::editor::registers::PasteDestination::Prompt,
                    );
                }
                Key::CtrlW | Key::CtrlA => self.cmdline_insert_word(key == Key::CtrlA),
                _ => {}
            }
            return;
        }
        match key {
            Key::Char(c) => {
                let mut buffer = [0u8; 4];
                line.insert(c.encode_utf8(&mut buffer));
                self.cmdline_edited();
            }
            key => {
                if let Some(action) = super::keys::action(key) {
                    self.cmdline_action(action);
                }
            }
        }
    }

    /// Literal text into the open line (bracketed paste, a register).
    pub(crate) fn cmdline_paste(&mut self, text: &str) {
        if !self.cmdline_live() {
            return;
        }
        if text.contains(['\r', '\n']) {
            self.message = "input line cannot contain a newline".into();
            return;
        }
        if let Some(line) = self.cmdline.line_mut() {
            line.insert(text);
            self.cmdline_edited();
        }
    }

    fn cmdline_action(&mut self, action: Action) {
        let Some(line) = self.cmdline.line_mut() else {
            return;
        };
        match action {
            Action::Run => {
                if let Some(line) = self.cmdline.close() {
                    self.run_cmdline(line);
                }
                return;
            }
            Action::Cancel => return self.cancel_cmdline(),
            Action::Backspace if line.is_empty() => return self.cancel_cmdline(),
            Action::Backspace => line.backspace(),
            Action::Delete => line.delete(),
            Action::DeleteWord => line.delete_word(),
            Action::DeleteToStart => line.delete_to_start(),
            Action::Left => return line.left(),
            Action::Right => return line.right(),
            Action::Home => return line.home(),
            Action::End => {
                if !complete::step(self, Step::Dismiss) {
                    if let Some(line) = self.cmdline.line_mut() {
                        line.end();
                    }
                }
                return;
            }
            Action::Register => return line.await_register(),
            Action::Repaint => {
                self.needs_repaint = true;
                return;
            }
            Action::Complete | Action::CompleteBack | Action::List | Action::Accept => {
                complete::step(
                    self,
                    match action {
                        Action::Complete => Step::Next,
                        Action::CompleteBack => Step::Previous,
                        Action::List => Step::List,
                        _ => Step::Accept,
                    },
                );
                return;
            }
            Action::Older | Action::Newer | Action::OlderPrefix | Action::NewerPrefix => {
                // the menu takes Ctrl-N/Ctrl-P while it shows, and Up/Down
                // once a row is selected; otherwise they walk history
                let menu = match action {
                    Action::Older | Action::Newer => self.cmdline_menu().is_some(),
                    _ => complete::selected(self),
                };
                let older = matches!(action, Action::Older | Action::OlderPrefix);
                if menu {
                    complete::step(self, if older { Step::Previous } else { Step::Next });
                    return;
                }
                let filtered = matches!(action, Action::OlderPrefix | Action::NewerPrefix);
                if self.cmdline.recall(older, filtered) {
                    self.cmdline_parse();
                    self.incsearch_jump();
                }
                return;
            }
            Action::Window => return,
        }
        self.cmdline_edited();
    }

    /// `Ctrl-R Ctrl-W` / `Ctrl-R Ctrl-A`: the word or WORD under the
    /// cursor of the pane the line opened from.
    fn cmdline_insert_word(&mut self, big: bool) {
        let Some(origin) = self.cmdline.line().map(|line| line.origin().clone()) else {
            return;
        };
        let Some(document) = self.docs.get(origin.pane.doc) else {
            return;
        };
        let buf = &document.buf;
        let head = origin.pane.sels.primary().head;
        let line = buf.line_of(head);
        let text = buf.line_text(line);
        let column = head - buf.line_start(line);
        let class = |c: char| {
            if big {
                !c.is_whitespace()
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };
        // vim: the word under or after the cursor on its line
        let Some(at) = text
            .char_indices()
            .find(|(at, c)| *at + c.len_utf8() > column && class(*c))
            .map(|(at, _)| at)
        else {
            return;
        };
        let start = text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| class(*c))
            .last()
            .map_or(at, |(start, _)| start);
        let end = text[at..]
            .char_indices()
            .find(|(_, c)| !class(*c))
            .map_or(text.len(), |(end, _)| at + end);
        let word = text[start..end].to_owned();
        self.cmdline_paste(&word);
    }

    /// The text changed: a history walk ends, the menu follows a `:`
    /// line, incsearch a pattern.
    fn cmdline_edited(&mut self) {
        if let Some(line) = self.cmdline.line_mut() {
            line.recall = None;
        }
        complete::typed(self);
        self.cmdline_parse();
        self.incsearch_jump();
    }

    /// Parse the line once for this revision of its text: the `:s` spec,
    /// the range's lines, the pattern. Everything after reads the parse.
    pub(crate) fn cmdline_parse(&mut self) {
        let Some(line) = self.cmdline.line() else {
            return;
        };
        let revision = Some(line.revision());
        if self.cmdline.parsed.revision == revision {
            return;
        }
        let head = line.origin().pane.sels.primary().head;
        let body = line.body();
        let mut parsed = super::parsed::Parsed {
            revision,
            ..Default::default()
        };
        if line.search().is_some() {
            parsed.search =
                (!body.is_empty()).then(|| grammar::CompiledQuery::compile(body, false));
        } else if matches!(line.context(), Context::Ex(_)) {
            // only a `:s` line is one (another command's bad range is
            // the range's error)
            let substitute = crate::editor::ex::complete::split::command(body)
                .is_some_and(|command| matches!(command.name(), "substitute" | "&" | "~"));
            if substitute {
                parsed.substitute =
                    crate::editor::ex::parse_substitute_line(self, body, head).transpose();
            }
            let typed = body.trim_start_matches([' ', ':']);
            if crate::editor::ex::complete::split::range_len(typed) > 0 {
                parsed.range = match crate::editor::ex::parse_ex_range(self, head, body) {
                    // a half-typed range may run backwards (`:3,` on line
                    // 1): the hint names the lines either way
                    Ok((Some(range), _)) => {
                        let (first, last) = range.lines();
                        Some(Ok((first.min(last), first.max(last))))
                    }
                    Ok((None, _)) => None,
                    Err(error) => Some(Err(error)),
                };
            }
        }
        self.cmdline.parsed = parsed;
    }

    fn run_cmdline(&mut self, line: Line) {
        if !self.restore_cmdline_origin(line.origin()) {
            self.fail("input cancelled: document or pane changed".into());
            return;
        }
        // Every typed line goes into history, a failed one too; a macro's
        // lines are not typed.
        if self.macro_depth == 0 {
            self.cmdline.remember(line.sigil(), line.body());
        }
        match line.context() {
            Context::Ex(_) => self.run_ex(line.body()),
            Context::Pipe { range, visual, .. } => {
                if self.buf().readonly {
                    self.fail("readonly buffer".into());
                    return;
                }
                self.pipe_run(range.start.get(), range.end.get(), line.body());
                if *visual {
                    self.mode = Mode::Normal;
                }
            }
            Context::Search { .. } => {
                let command = match self.search_line_command(&line, true) {
                    Ok(Some(command)) => command,
                    Ok(None) => return,
                    Err(error) => {
                        self.fail(error);
                        return;
                    }
                };
                if self.defer_resolution(
                    &command,
                    self.all_cursors(),
                    crate::editor::resolution::ResolutionPurpose::Execute,
                ) {
                    return;
                }
                // Runtime query errors must be discovered for every
                // cursor BEFORE dispatch changes last_search, history
                // or a register.
                if let Err(error) = self.search_line_heads(&line, &command) {
                    self.fail(error);
                    return;
                }
                self.dispatch_grammar(&command);
            }
        }
    }

    /// The typed command a search line currently stands for. An empty
    /// body with `repeat_empty` reuses the last compiled query (vim:
    /// `/⏎` / `?⏎`); the count/register/operator stay those of THIS
    /// entry. Direction comes from the line's own sigil.
    pub(crate) fn search_line_command(
        &self,
        line: &Line,
        repeat_empty: bool,
    ) -> Result<Option<Command>, String> {
        let Some((_, state)) = line.search() else {
            return Ok(None);
        };
        let query = if line.body().is_empty() {
            if !repeat_empty {
                return Ok(None);
            }
            self.last_search
                .as_ref()
                .ok_or_else(|| "no previous search".to_string())?
                .query
                .clone()
        } else if let Some(Some(parsed)) = (self.cmdline.parsed.revision == Some(line.revision()))
            .then_some(self.cmdline.parsed.search.as_ref())
        {
            // the open line's pattern, compiled once for this revision
            parsed.clone().map_err(|e| e.to_string())?
        } else {
            grammar::CompiledQuery::compile(line.body(), false).map_err(|e| e.to_string())?
        };
        let target = if line.backward() == Some(true) {
            grammar::Motion::SearchBackward(query)
        } else {
            grammar::Motion::Search(query)
        };
        Ok(Some(Command {
            op: state.op,
            register: state.register,
            count: state.count(),
            target: grammar::Target::Motion(target),
            // Execution records the typed command for dot repeat.
            force: None,
            keys: String::new(),
        }))
    }

    /// Where every cursor of the saved origin lands for this command —
    /// the exact execution resolver (`resolve_many`), so incsearch and
    /// Enter can never disagree.
    fn search_line_heads(&self, line: &Line, command: &Command) -> Result<Vec<usize>, String> {
        let Some((origin, _)) = line.search() else {
            return Ok(Vec::new());
        };
        let heads = origin.pane.sels.heads();
        let resolved = self.resolved_many(command, &heads)?;
        Ok(heads
            .into_iter()
            .zip(resolved)
            .map(|(head, hit)| {
                hit.map_or(head, |hit| {
                    self.clamp_pos(grammar::cursor_after(self.buf(), head, command, &hit))
                })
            })
            .collect())
    }

    /// Live incsearch (vim parity): while a `/`/`?` line is open every
    /// cursor tracks the pattern's match from the saved origin — typing
    /// AND deleting re-resolve, all cursors at once. No match parks at
    /// the origin (vim keeps position and reports E486).
    pub(crate) fn incsearch_jump(&mut self) {
        let Some(line) = self.cmdline.line().filter(|line| line.search().is_some()) else {
            return;
        };
        let line = line.clone();
        let Some((origin, _)) = line.search() else {
            return;
        };
        if !self.cmdline_origin_valid(origin) {
            return;
        }
        let command = self.search_line_command(&line, false);
        self.restore_cmdline_origin(origin);
        let command = match command {
            Ok(Some(command)) => command,
            Ok(None) => {
                self.resolution.cancel_preview();
                return;
            }
            Err(error) => {
                self.resolution.cancel_preview();
                self.message = error;
                return;
            }
        };
        if self.defer_resolution(
            &command,
            origin.pane.sels.heads(),
            crate::editor::resolution::ResolutionPurpose::IncSearch,
        ) {
            return;
        }
        match self.search_line_heads(&line, &command) {
            Ok(heads) => {
                let mut heads = heads.into_iter();
                if let Some(head) = heads.next() {
                    self.set_head(head);
                }
                self.sels_mut().set_extras(heads);
                self.clamp_cursor();
            }
            Err(error) => self.message = error,
        }
    }

    /// Pending search pattern (incsearch highlight), if any: the `/` or
    /// `?` line's body. Pipe and ex bodies never misread as patterns.
    pub fn search_pattern(&self) -> Option<&str> {
        let line = self.cmdline.line()?;
        line.search()?;
        (!line.body().is_empty()).then_some(line.body())
    }
}
