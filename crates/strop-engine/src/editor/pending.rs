//! One owner for modal input (R7). A text prompt — `:ex`, `/search`,
//! `?search`, `|pipe` — owns its `LineEdit` plus everything about the
//! moment it opened: the pane (full selections and viewport), the
//! document and its revision. Structural operator composition stays in
//! the `Walker`; prompts never fabricate grammar text.

use strop_core::{id::BufferRevision, Range};
use strop_picker::LineEdit;

use super::{input::ParserState, panes::Pane, Key};

/// Where a prompt opened: the whole originating pane (document,
/// complete selections, viewport) and the buffer revision. Cancellation
/// restores this verbatim; acceptance validates it before executing.
#[derive(Debug, Clone)]
pub(crate) struct SearchOrigin {
    pub pane_index: usize,
    pub pane: Pane,
    pub revision: BufferRevision,
}

/// What the prompt is for. The sigil is derivable; the context is not.
#[derive(Debug, Clone)]
pub(crate) enum PromptContext {
    Ex(SearchOrigin),
    Search {
        origin: SearchOrigin,
        /// The typed entry state (counts/register/operator) that
        /// survived crossing into the prompt (`2d/foo` keeps 2 and d).
        state: ParserState,
        backward: bool,
    },
    Pipe {
        origin: SearchOrigin,
        range: Range,
        visual: bool,
    },
}

/// One open modal line: the editable text plus its origin context.
#[derive(Debug, Clone)]
pub(crate) struct TextPrompt {
    line: LineEdit,
    context: PromptContext,
}

impl TextPrompt {
    pub(crate) fn new(context: PromptContext) -> Self {
        let sigil = match &context {
            PromptContext::Ex(_) => ':',
            PromptContext::Search {
                backward: false, ..
            } => '/',
            PromptContext::Search { backward: true, .. } => '?',
            PromptContext::Pipe { .. } => '|',
        };
        Self {
            line: LineEdit::new(sigil.to_string()),
            context,
        }
    }

    pub(crate) fn sigil(&self) -> char {
        match &self.context {
            PromptContext::Ex(_) => ':',
            PromptContext::Search {
                backward: false, ..
            } => '/',
            PromptContext::Search { backward: true, .. } => '?',
            PromptContext::Pipe { .. } => '|',
        }
    }

    /// The text after the sigil — the command/pattern body.
    pub(crate) fn body(&self) -> &str {
        &self.line.text()[1..]
    }

    /// The full line, sigil included (empty string semantics live on
    /// `PendingInput`, which knows whether a prompt is open at all).
    pub(crate) fn text(&self) -> &str {
        self.line.text()
    }

    /// Caret byte offset into `text()` (sigil-inclusive).
    pub(crate) fn cursor(&self) -> usize {
        self.line.cursor()
    }

    /// True when Esc put the line's own caret into normal mode.
    pub(crate) fn normal(&self) -> bool {
        self.line.normal
    }

    pub(crate) fn context(&self) -> &PromptContext {
        &self.context
    }

    pub(crate) fn origin(&self) -> &SearchOrigin {
        match &self.context {
            PromptContext::Ex(origin)
            | PromptContext::Search { origin, .. }
            | PromptContext::Pipe { origin, .. } => origin,
        }
    }

    /// The saved search entry, when this prompt is a `/` or `?` line.
    pub(crate) fn search(&self) -> Option<(&SearchOrigin, &ParserState)> {
        match &self.context {
            PromptContext::Search { origin, state, .. } => Some((origin, state)),
            _ => None,
        }
    }

    pub(crate) fn backward(&self) -> Option<bool> {
        match &self.context {
            PromptContext::Search { backward, .. } => Some(*backward),
            _ => None,
        }
    }
}

/// The editor's one prompt slot: open or closed, nothing in between.
/// The field machine rides alongside: key-sequence state (a pending
/// operator) survives between the prompt's normal-mode key events.
#[derive(Debug, Default)]
pub struct PendingInput {
    active: Option<TextPrompt>,
    machine: super::field::FieldMachine,
}

/// One input event for the open prompt.
pub(crate) enum PendingEvent {
    Key(Key),
    /// Bracketed paste routed away from the document (literal text).
    Paste(String),
    /// Ex completion offered the given body (Tab cycling).
    CompleteEx(String),
    Cancel,
}

/// What the reducer did — the caller owns every side effect.
pub(crate) enum PendingEffect {
    None,
    /// Text changed: re-resolve incsearch.
    Edited,
    /// The line's modal mode flipped.
    ModeChanged,
    /// Tab on the ex line: cycle completion.
    CompleteEx,
    /// Ctrl-L: terminal desync recovery.
    Repaint,
    /// The event was refused (a pasted newline, a key the field's
    /// grammar does not admit) — the message is the feedback.
    Rejected(String),
    /// Enter: consume and execute (Pipe/Search/Ex by context).
    Accepted(TextPrompt),
    /// Esc-Esc / sigil deletion / Cancel: consume and restore.
    Aborted(TextPrompt),
}

impl PendingInput {
    pub(crate) fn prompt(&self) -> Option<&TextPrompt> {
        self.active.as_ref()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The full line, sigil included; empty when no prompt is open.
    pub fn text(&self) -> &str {
        self.prompt().map_or("", TextPrompt::text)
    }

    /// Caret byte offset into `text()`; 0 when no prompt is open.
    pub fn cursor(&self) -> usize {
        self.prompt().map_or(0, TextPrompt::cursor)
    }

    pub(crate) fn normal(&self) -> bool {
        self.prompt().is_some_and(TextPrompt::normal)
    }

    /// The open prompt's sigil (`: / ? |`); None when closed.
    pub(crate) fn sigil(&self) -> Option<char> {
        self.prompt().map(TextPrompt::sigil)
    }

    /// Open a prompt. Opening while another is active is a caller bug:
    /// the previous origin would be silently discarded. The field
    /// machine grounds here, so a dead prompt's pending operator never
    /// leaks into the next line.
    pub(crate) fn open(&mut self, prompt: TextPrompt) {
        assert!(
            self.active.is_none(),
            "cancel the previous prompt before opening another"
        );
        self.machine.clear();
        self.active = Some(prompt);
    }

    /// The one reducer. Every surface funnels here; effects are the
    /// caller's to apply. Enter and abort consume the prompt and hand
    /// it back — exactly once (a closed prompt yields `None`).
    pub(crate) fn reduce(&mut self, event: PendingEvent) -> PendingEffect {
        let Some(prompt) = self.active.as_mut() else {
            return PendingEffect::None;
        };
        if matches!(event, PendingEvent::Cancel)
            || matches!(event, PendingEvent::Key(Key::Esc)) && prompt.line.normal
        {
            return PendingEffect::Aborted(self.active.take().expect("active prompt"));
        }
        if matches!(event, PendingEvent::Key(Key::Enter)) {
            return PendingEffect::Accepted(self.active.take().expect("active prompt"));
        }
        let old_revision = prompt.line.revision();
        let old_normal = prompt.line.normal;
        match event {
            PendingEvent::Paste(text) => {
                if text.contains(['\r', '\n']) {
                    return PendingEffect::Rejected("input line cannot contain a newline".into());
                }
                prompt.line.insert_str(&text);
            }
            PendingEvent::CompleteEx(body) if prompt.sigil() == ':' => {
                prompt.line.set_text(format!(":{body}"));
                return PendingEffect::Edited;
            }
            PendingEvent::CompleteEx(_) | PendingEvent::Cancel => return PendingEffect::None,
            PendingEvent::Key(Key::Esc) => {
                prompt.line.normal = true;
                prompt.line.set_cursor(usize::MAX);
            }
            PendingEvent::Key(Key::Backspace) if prompt.line.normal => {
                // vim: BS in normal mode is h — a pure grammar motion
                let _ = self.machine.feed(&mut prompt.line, Key::Char('h'));
            }
            PendingEvent::Key(Key::Backspace) => {
                prompt.line.backspace();
            }
            PendingEvent::Key(Key::Char(c)) if prompt.line.normal => {
                // the real vim grammar (0003 §2): the field machine
                // resolves every sequence against the line's buffer
                if let super::field::FieldReply::Refused(message) =
                    self.machine.feed(&mut prompt.line, Key::Char(c))
                {
                    return PendingEffect::Rejected(message);
                }
            }
            PendingEvent::Key(Key::Char(c)) => {
                prompt.line.insert_char(c);
            }
            PendingEvent::Key(Key::Left) => prompt.line.move_left(),
            PendingEvent::Key(Key::Right) => prompt.line.move_right(),
            PendingEvent::Key(Key::Tab) if prompt.sigil() == ':' => {
                return PendingEffect::CompleteEx;
            }
            PendingEvent::Key(Key::CtrlL) => return PendingEffect::Repaint,
            PendingEvent::Key(_) => return PendingEffect::None,
        }
        // The sigil is structural: an edit that removes it (backspace
        // at 1, `x` at 0, an operator range covering the sigil) closes
        // the prompt — same gesture as Esc-Esc.
        if !prompt.line.text().starts_with(prompt.sigil()) {
            return PendingEffect::Aborted(self.active.take().expect("active prompt"));
        }
        if old_revision != prompt.line.revision() {
            PendingEffect::Edited
        } else if old_normal != prompt.line.normal {
            PendingEffect::ModeChanged
        } else {
            PendingEffect::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use strop_core::Buffer;

    #[test]
    fn accepting_a_visual_pipe_returns_its_original_range_and_literal_command() {
        let e = Editor::new(Buffer::from_text("first\nsecond\n"));
        let origin = SearchOrigin {
            pane_index: e.active_pane,
            pane: e.view().clone(),
            revision: e.buf().revision(),
        };
        let mut pending = PendingInput::default();
        pending.open(TextPrompt::new(PromptContext::Pipe {
            origin,
            range: Range::charwise(0, 13),
            visual: true,
        }));
        for c in "cat".chars() {
            pending.reduce(PendingEvent::Key(Key::Char(c)));
        }
        let PendingEffect::Accepted(prompt) = pending.reduce(PendingEvent::Key(Key::Enter)) else {
            panic!("pipe not accepted");
        };
        assert_eq!(prompt.body(), "cat");
        match prompt.context() {
            PromptContext::Pipe { range, visual, .. } => {
                assert_eq!(*range, Range::charwise(0, 13));
                assert!(*visual);
            }
            _ => panic!("lost pipe effect"),
        }
        assert!(!pending.is_active());
        // exactly-once consumption: a second Enter finds nothing open
        assert!(matches!(
            pending.reduce(PendingEvent::Key(Key::Enter)),
            PendingEffect::None
        ));
    }

    #[test]
    fn esc_once_is_modal_twice_aborts_and_sigil_deletion_aborts() {
        let e = Editor::new(Buffer::from_text("x\n"));
        let origin = SearchOrigin {
            pane_index: e.active_pane,
            pane: e.view().clone(),
            revision: e.buf().revision(),
        };
        let mut pending = PendingInput::default();
        pending.open(TextPrompt::new(PromptContext::Search {
            origin,
            state: ParserState::default(),
            backward: false,
        }));
        for c in "ab".chars() {
            pending.reduce(PendingEvent::Key(Key::Char(c)));
        }
        assert_eq!(pending.text(), "/ab");
        // first Esc: the line's caret goes modal, prompt stays open
        assert!(matches!(
            pending.reduce(PendingEvent::Key(Key::Esc)),
            PendingEffect::ModeChanged
        ));
        assert!(pending.normal());
        // modal x at the sigil deletes it: abort
        let _ = pending.reduce(PendingEvent::Key(Key::Char('0')));
        let PendingEffect::Aborted(prompt) = pending.reduce(PendingEvent::Key(Key::Char('x')))
        else {
            panic!("sigil deletion must abort");
        };
        assert_eq!(prompt.text(), "ab"); // sigil gone, body intact
        assert!(!pending.is_active());
        assert_eq!(pending.text(), "");
    }

    #[test]
    fn paste_is_literal_and_newlines_are_rejected() {
        let e = Editor::new(Buffer::from_text("x\n"));
        let origin = SearchOrigin {
            pane_index: e.active_pane,
            pane: e.view().clone(),
            revision: e.buf().revision(),
        };
        let mut pending = PendingInput::default();
        pending.open(TextPrompt::new(PromptContext::Ex(origin)));
        assert!(matches!(
            pending.reduce(PendingEvent::Paste("w q".into())),
            PendingEffect::Edited
        ));
        assert_eq!(pending.text(), ":w q");
        assert!(matches!(
            pending.reduce(PendingEvent::Paste("\nx".into())),
            PendingEffect::Rejected(message) if message == "input line cannot contain a newline"
        ));
        assert_eq!(pending.text(), ":w q");
    }

    /// Body typed on the `:` line, Esc into the line's normal mode,
    /// then edited with the given keys: the resulting line text.
    fn ex_after(body: &str, keys: &str) -> String {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":");
        e.feed_text(body);
        e.feed(crate::editor::Key::Esc);
        e.feed_text(keys);
        e.pending.text().to_string()
    }

    #[test]
    fn ex_line_normal_mode_is_the_real_grammar() {
        // the same grammar machine as the picker fields (0003 §2)
        assert_eq!(ex_after("w foo", "bdw"), ":w ");
        assert_eq!(ex_after("w foo", "db"), ":w ");
        // de on a one-char word: e sits at the word's end already, so
        // it extends to the next word's end (vim does the same)
        assert_eq!(ex_after("w foo", "0lde"), ":");
        assert_eq!(ex_after("w:x", "0ldf:"), ":x");
        assert_eq!(ex_after("a b c", "0l2dw"), ":c");
        assert_eq!(ex_after("w foo", "bdiw"), ":w ");
        // j/k are motions that cannot leave the one line: honest no-ops
        assert_eq!(ex_after("w", "jk"), ":w");
    }

    #[test]
    fn ex_line_change_reenters_insert_and_sigil_ops_abort() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":w foo");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("bcwbar");
        assert_eq!(e.pending.text(), ":w bar");
        assert!(!e.pending.normal(), "cw re-enters the line's insert mode");
        // an operator range covering the sigil trips the standing
        // sigil rule: the prompt closes, exactly like x at 0
        e.feed(crate::editor::Key::Esc);
        assert!(e.pending.normal());
        e.feed_text("0dw");
        assert!(!e.pending.is_active(), "0dw covers the sigil: abort");
    }
}
