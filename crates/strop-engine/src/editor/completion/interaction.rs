use super::context::Invocation;
use super::model::CandidateKey;
use super::session::{
    Acceptance, Direction, InsertPrefix, ProviderState, RequestIntent, ResolveState, Selection,
};
use crate::editor::{Editor, InputOwner, Mode};
use strop_core::frontend_input::Key;
use strop_core::worker::CancelReason;
use strop_lsp::completion::CompletionTrigger;

pub(crate) enum KeyDisposition {
    Requested,
    Navigated,
    Accepted,
    Dismissed,
    DismissAndForward,
    PassThrough,
}

impl Editor {
    pub(crate) fn completion_normalize_key(&self, key: Key) -> Key {
        let document_insert = self.mode == Mode::Insert
            && matches!(
                self.input_owner(),
                InputOwner::Document | InputOwner::HoverCard | InputOwner::BlameCard
            );
        if document_insert {
            return key;
        }
        // Preserve the pre-completion field/normal-mode normalization. Physical
        // PTY input was already routed before editor-key normalization.
        match key {
            Key::CtrlN => Key::Down,
            Key::CtrlP => Key::Up,
            Key::CtrlE => Key::Char('e'),
            Key::CtrlY => Key::Char('y'),
            other => other,
        }
    }

    pub(crate) fn completion_key(&mut self, key: Key) -> KeyDisposition {
        self.completion_refresh_language_owner();
        if matches!(self.completion.prefix, InsertPrefix::ControlX) {
            self.completion.prefix = InsertPrefix::None;
            if key == Key::CtrlO {
                self.completion_request(Invocation::ManualLanguage, None);
                return KeyDisposition::Requested;
            }
        }
        match key {
            Key::CtrlSpace => {
                self.completion_request(Invocation::ManualCombined, None);
                KeyDisposition::Requested
            }
            Key::CtrlX => {
                self.completion_close(CancelReason::Dismissed);
                self.completion.prefix = InsertPrefix::ControlX;
                KeyDisposition::Requested
            }
            Key::CtrlN | Key::CtrlP => {
                let direction = if key == Key::CtrlN {
                    Direction::Next
                } else {
                    Direction::Previous
                };
                if self.completion.session.is_some() {
                    self.completion_navigate(direction);
                } else {
                    self.completion_request(Invocation::ManualWords, Some(direction));
                }
                KeyDisposition::Navigated
            }
            Key::Up | Key::Down
                if self
                    .completion
                    .session
                    .as_ref()
                    .is_some_and(|session| session.count() > 0) =>
            {
                self.completion_navigate(if key == Key::Down {
                    Direction::Next
                } else {
                    Direction::Previous
                });
                KeyDisposition::Navigated
            }
            Key::CtrlY => {
                self.completion_accept_selected();
                KeyDisposition::Accepted
            }
            Key::Enter | Key::Tab
                if self
                    .completion
                    .session
                    .as_ref()
                    .is_some_and(|session| session.selected().is_some()) =>
            {
                self.completion_accept_selected();
                KeyDisposition::Accepted
            }
            Key::CtrlE => {
                self.completion_close(CancelReason::Dismissed);
                KeyDisposition::Dismissed
            }
            Key::CtrlL => KeyDisposition::PassThrough,
            Key::Esc => {
                self.completion_close(CancelReason::Dismissed);
                KeyDisposition::DismissAndForward
            }
            Key::Char(_) | Key::Backspace => {
                self.completion_typing(key);
                KeyDisposition::PassThrough
            }
            _ => {
                self.completion_close(CancelReason::Dismissed);
                KeyDisposition::PassThrough
            }
        }
    }

    fn completion_request(&mut self, invocation: Invocation, direction: Option<Direction>) {
        self.completion_close(CancelReason::Superseded);
        if !self.config.completion.enabled {
            self.message = "code completion is disabled".into();
            return;
        }
        self.completion.intent = Some(RequestIntent {
            invocation,
            direction,
            trigger: CompletionTrigger::Invoked,
            cached_language: None,
            selection: None,
        });
    }

    fn completion_typing(&mut self, key: Key) {
        self.completion.prefix_growth = None;
        if !self.config.completion.enabled {
            return;
        }
        if self.completion.session.as_ref().is_some_and(|session| {
            !session
                .query
                .context
                .still_owns(self, self.completion.settings_generation)
        }) {
            self.completion_close(CancelReason::Superseded);
        }
        let invocation = self
            .completion
            .session
            .as_ref()
            .map(|session| session.query.context.invocation)
            .or_else(|| {
                self.completion
                    .intent
                    .as_ref()
                    .map(|intent| intent.invocation)
            });
        let character = match key {
            Key::Char(character) => Some(character),
            _ => None,
        };
        let trigger = character
            .filter(|character| !strop_grammar::is_keyword(*character))
            .filter(|character| self.completion_declared_trigger(*character));
        if character.is_some_and(|character| !strop_grammar::is_keyword(character))
            && trigger.is_none()
        {
            self.completion_close(CancelReason::Superseded);
            return;
        }
        let Some(invocation) = invocation.or_else(|| {
            self.config
                .completion
                .auto_popup
                .then_some(Invocation::Automatic)
        }) else {
            return;
        };
        let mut request = RequestIntent {
            invocation,
            direction: None,
            trigger: trigger.map_or(CompletionTrigger::Invoked, CompletionTrigger::Character),
            cached_language: None,
            selection: None,
        };
        if let Some(session) = &self.completion.session {
            if trigger.is_none()
                && session
                    .language()
                    .is_some_and(|language| language.items.server_incomplete)
            {
                request.trigger = CompletionTrigger::Incomplete;
            }
            if matches!(key, Key::Char(character) if strop_grammar::is_keyword(character)) {
                if let ProviderState::Settled(delivery) = &session.language {
                    if session
                        .language()
                        .is_some_and(|language| language.reusable())
                    {
                        request.cached_language = Some(delivery.clone());
                    }
                }
            }
            if let Selection::Chosen { key, .. } = &session.selection {
                request.selection = Some(key.clone());
            }
        }
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.query(None);
        }
        let requests = self
            .completion
            .session
            .as_ref()
            .map(|session| [session.request, session.resolve.request()])
            .unwrap_or([None, None]);
        for stamp in requests.into_iter().flatten() {
            self.completion_cancel_request(stamp, CancelReason::Superseded);
        }
        if let Some(session) = &mut self.completion.session {
            session.request = None;
            session.resolve = ResolveState::Idle;
            session.documentation = None;
            session.preparation = None;
            session.acceptance = Acceptance::Idle;
        }
        self.completion.intent = Some(request);
        self.completion.released();
    }

    fn completion_navigate(&mut self, direction: Direction) {
        let Some(session) = &mut self.completion.session else {
            return;
        };
        let count = session.count();
        if count == 0 {
            session.selection = Selection::Awaiting(direction);
            return;
        }
        let index = match (session.selected().map(|(index, _)| index), direction) {
            (Some(index), Direction::Next) => (index + 1) % count,
            (Some(index), Direction::Previous) => (index + count - 1) % count,
            (None, Direction::Next) => 0,
            (None, Direction::Previous) => count - 1,
        };
        self.completion_choose(index);
    }

    fn completion_choose(&mut self, index: usize) {
        let Some(session) = &self.completion.session else {
            return;
        };
        let Some(candidate) = session.candidate(index) else {
            return;
        };
        let key = candidate.key();
        let generation = match self.worker_ids.allocate() {
            Ok(generation) => generation,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let resolve = session.resolve.request();
        if let Some(stamp) = resolve {
            self.completion_cancel_request(stamp, CancelReason::Superseded);
        }
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.select(Some(generation));
        }
        if let Some(session) = &mut self.completion.session {
            session.selection = Selection::Chosen { key, generation };
            session.resolve = ResolveState::Idle;
            session.documentation = None;
            session.preparation = None;
            session.acceptance = Acceptance::Idle;
        }
        self.completion.released();
        self.completion_prepare_documentation();
        self.completion_resolve_selected();
    }

    pub(super) fn completion_reconcile_selection(&mut self) {
        let Some(session) = &self.completion.session else {
            return;
        };
        match &session.selection {
            Selection::Awaiting(direction) if session.count() > 0 => {
                self.completion_choose(match direction {
                    Direction::Next => 0,
                    Direction::Previous => session.count() - 1,
                });
            }
            Selection::Chosen { key, .. }
                if session.selected().is_none()
                    && match key {
                        CandidateKey::Word(_) => !session.words.pending(),
                        CandidateKey::Language { .. } => !session.language.pending(),
                    } =>
            {
                let resolve = session.resolve.request();
                if let Some(stamp) = resolve {
                    self.completion_cancel_request(stamp, CancelReason::Superseded);
                }
                if let Some(mailbox) = self.completion.worker.mailbox() {
                    mailbox.select(None);
                }
                if let Some(session) = &mut self.completion.session {
                    session.selection = Selection::None;
                    session.resolve = ResolveState::Idle;
                    session.documentation = None;
                    session.preparation = None;
                    session.acceptance = Acceptance::Idle;
                }
            }
            _ => {}
        }
        let details_needed = self.completion.session.as_ref().is_some_and(|session| {
            session.preparation.is_none()
                && session.documentation.is_none()
                && matches!(session.acceptance, Acceptance::Idle)
                && matches!(
                    session.selected(),
                    Some((_, super::session::Candidate::Language { .. }))
                )
        });
        if details_needed {
            self.completion_prepare_documentation();
            if self
                .completion
                .session
                .as_ref()
                .is_some_and(|session| matches!(session.resolve, ResolveState::Idle))
            {
                self.completion_resolve_selected();
            }
        }
    }

    fn completion_declared_trigger(&mut self, character: char) -> bool {
        let Some((source, _)) = self.source_edit_position(self.current(), self.head()) else {
            return false;
        };
        let Some(binding) = self.lsp_state.bindings.get(&source) else {
            return false;
        };
        let server = binding.server;
        match self
            .tape
            .call("completion.trigger", &(server, character), || {
                self.lsp_live_client(server)
                    .is_some_and(|client| client.caps().completion_trigger(character))
            }) {
            Ok(trigger) => trigger,
            Err(error) => {
                self.message = format!("completion trigger diverged: {error}");
                false
            }
        }
    }
}
