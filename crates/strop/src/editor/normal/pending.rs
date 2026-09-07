//! normal/pending.rs — prompt effects (R7): one reducer consumer,
//! independent of the surface that opened the prompt. Editing,
//! incsearch, acceptance and cancellation all live here; the surfaces
//! only route keys to the shared `PendingInput`.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Parse};

use crate::editor::pending::{
    PendingEffect, PendingEvent, PromptContext, SearchOrigin, TextPrompt,
};
use crate::editor::{input::ParserState, Editor, Key, Mode};

impl Editor {
    /// A text line opened (`: / ? |`) with the typed entry state that
    /// survived the crossing — counts, register, operator.
    pub(crate) fn begin_text_line(&mut self, sigil: char, state: ParserState) {
        self.cancel_pending();
        let origin = SearchOrigin {
            pane_index: self.active_pane,
            pane: self.view().clone(),
            revision: self.buf().revision(),
        };
        let context = match sigil {
            ':' => PromptContext::Ex(origin),
            '/' | '?' => PromptContext::Search {
                origin,
                state,
                backward: sigil == '?',
            },
            '|' => {
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    return;
                }
                let visual = matches!(
                    self.mode,
                    Mode::Visual | Mode::VisualLine | Mode::VisualBlock
                );
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
                PromptContext::Pipe {
                    origin,
                    range,
                    visual,
                }
            }
            _ => unreachable!("Walker only emits text-line sigils"),
        };
        self.pending.open(TextPrompt::new(context));
    }

    /// The saved origin still describes this editor: same pane slot,
    /// same document incarnation, unchanged revision. Service results
    /// and edits invalidate it; delivery-time checks are the backstop.
    pub(crate) fn pending_origin_valid(&self, origin: &SearchOrigin) -> bool {
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

    fn restore_prompt_origin(&mut self, origin: &SearchOrigin) -> bool {
        if !self.pending_origin_valid(origin) {
            return false;
        }
        // Never clamp/normalize: those operations would change saved
        // anchors, duplicate cursors or a deliberately parked viewport.
        self.panes[origin.pane_index] = origin.pane.clone();
        true
    }

    /// Abort the open prompt (if any), restoring its origin first.
    /// Called before anything replaces the pane/document/revision the
    /// prompt was opened against — never on rejected service results.
    pub(crate) fn cancel_pending(&mut self) {
        if let PendingEffect::Aborted(prompt) = self.pending.reduce(PendingEvent::Cancel) {
            self.restore_prompt_origin(prompt.origin());
        }
    }

    pub(crate) fn feed_pending(&mut self, key: Key) {
        self.feed_pending_event(PendingEvent::Key(key));
    }

    /// The shared prompt entrypoint: keys, pastes and completion events
    /// all reduce through the same owner.
    pub(crate) fn feed_pending_event(&mut self, event: PendingEvent) {
        if self
            .pending
            .prompt()
            .is_some_and(|p| !self.pending_origin_valid(p.origin()))
        {
            self.cancel_pending();
            self.message = "input cancelled: document or pane changed".into();
            return;
        }
        match self.pending.reduce(event) {
            PendingEffect::None | PendingEffect::ModeChanged => {}
            PendingEffect::Edited => self.incsearch_jump(),
            PendingEffect::CompleteEx => self.ex_tab_complete(),
            PendingEffect::Repaint => self.needs_repaint = true,
            PendingEffect::Rejected(error) => self.message = error.into(),
            PendingEffect::Aborted(prompt) => {
                self.restore_prompt_origin(prompt.origin());
            }
            PendingEffect::Accepted(prompt) => self.accept_prompt(prompt),
        }
    }

    fn accept_prompt(&mut self, prompt: TextPrompt) {
        if !self.restore_prompt_origin(prompt.origin()) {
            self.message = "input cancelled: document or pane changed".into();
            return;
        }
        match prompt.context() {
            PromptContext::Ex(_) => self.run_ex(prompt.body()),
            PromptContext::Pipe { range, visual, .. } => {
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    return;
                }
                self.pipe_run(range.start.get(), range.end.get(), prompt.body());
                if *visual {
                    self.mode = Mode::Normal;
                }
            }
            PromptContext::Search { .. } => {
                let command = match self.search_prompt_command(&prompt, true) {
                    Ok(Some(command)) => command,
                    Ok(None) => return,
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                };
                // Runtime query errors must be discovered for every
                // cursor BEFORE dispatch changes last_search, history
                // or a register.
                if let Err(error) = self.search_prompt_heads(&prompt, &command) {
                    self.message = error;
                    return;
                }
                self.dispatch_grammar(&command);
            }
        }
    }

    /// The typed command a search prompt currently stands for. An
    /// empty body with `repeat_empty` reuses the last compiled query
    /// (vim: `/⏎` / `?⏎`); the count/register/operator stay those of
    /// THIS entry. Direction comes from the prompt's own sigil.
    pub(crate) fn search_prompt_command(
        &self,
        prompt: &TextPrompt,
        repeat_empty: bool,
    ) -> Result<Option<Command>, String> {
        let Some((_, state)) = prompt.search() else {
            return Ok(None);
        };
        let query = if prompt.body().is_empty() {
            if !repeat_empty {
                return Ok(None);
            }
            self.last_search
                .as_ref()
                .ok_or_else(|| "no previous search".to_string())?
                .query
                .clone()
        } else {
            grammar::CompiledQuery::compile(prompt.body(), false).map_err(|e| e.to_string())?
        };
        let target = if prompt.backward() == Some(true) {
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
            keys: String::new(),
        }))
    }

    /// Where every cursor of the saved origin lands for this command —
    /// the exact execution resolver (`resolve_many`), so incsearch and
    /// Enter can never disagree.
    fn search_prompt_heads(
        &self,
        prompt: &TextPrompt,
        command: &Command,
    ) -> Result<Vec<usize>, String> {
        let (origin, _) = prompt.search().expect("search context");
        let heads = origin.pane.sels.heads();
        let resolved =
            grammar::resolve_many(self.buf(), &heads, command).map_err(|e| e.to_string())?;
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

    /// Live incsearch (vim parity): while a `/`/`?` prompt is open every
    /// cursor tracks the pattern's match from the saved origin — typing
    /// AND deleting re-resolve, all cursors at once. No match parks at
    /// the origin (vim keeps position and reports E486).
    pub(crate) fn incsearch_jump(&mut self) {
        let Some(prompt) = self.pending.prompt() else {
            return;
        };
        let Some((origin, _)) = prompt.search() else {
            return;
        };
        if !self.pending_origin_valid(origin) {
            return;
        }
        let result = self
            .search_prompt_command(prompt, false)
            .and_then(|command| {
                command
                    .map(|command| self.search_prompt_heads(prompt, &command))
                    .transpose()
            });
        let origin = origin.clone();
        self.restore_prompt_origin(&origin);
        match result {
            Ok(Some(heads)) => {
                let mut heads = heads.into_iter();
                self.set_head(heads.next().expect("primary selection"));
                self.sels_mut().set_extras(heads);
                self.clamp_cursor();
            }
            Ok(None) => {}
            Err(error) => self.message = error,
        }
    }

    /// Pending search pattern (incsearch highlight), if any: the `/` or
    /// `?` prompt's body. Pipe and ex bodies never misread as patterns.
    pub fn search_pattern(&self) -> Option<&str> {
        let prompt = self.pending.prompt()?;
        prompt.search()?;
        (!prompt.body().is_empty()).then_some(prompt.body())
    }

    /// vim Enter: [count] lines down, first non-blank. With the blame
    /// gutter on, Enter dives into the line's commit instead (0011 §3).
    pub fn enter_pub(&mut self) {
        if self.dive_from_blame() {
            return;
        }
        let n = self.walker.state.count1.unwrap_or(1);
        let line = (self.buf().line_of(self.head()) + n).min(self.buf().last_content_line());
        let s = self.buf().line_start(line);
        let e = self.buf().line_end(line);
        let mut p = s;
        while p < e
            && self
                .buf()
                .byte_at(p)
                .is_some_and(|b| b == b' ' || b == b'\t')
        {
            p += 1;
        }
        self.set_head(p.min(e));
        self.clamp_cursor();
    }

    pub(crate) fn run_motion(&mut self, keys: &str) {
        match grammar::parse(keys) {
            Parse::Complete(cmd) => self.move_cursor(&cmd),
            Parse::QueryError(error) => self.message = error.to_string(),
            Parse::Incomplete | Parse::Invalid => {}
        }
    }
}
