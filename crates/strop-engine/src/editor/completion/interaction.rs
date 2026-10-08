use super::context::Invocation;
use super::model::CandidateKey;
use super::session::{
    Acceptance, Candidate, Direction, InsertPrefix, ProviderState, RequestIntent, ResolveState,
    Selection,
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
        // Any key outside the Tab cycling pair retires an active live
        // preview before its normal handling (0059 §6.1).
        if !matches!(key, Key::Tab | Key::Backtab) {
            self.completion_revert_preview();
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
            Key::Tab | Key::Backtab
                if self
                    .completion
                    .session
                    .as_ref()
                    .is_some_and(|session| session.count() > 0) =>
            {
                let direction = if key == Key::Tab {
                    Direction::Next
                } else {
                    Direction::Previous
                };
                self.completion_preview_cycle(direction);
                KeyDisposition::Navigated
            }
            Key::Enter
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
            Key::Char(ch) => {
                // 0069 D05 commit characters (LSP
                // `completionItem.commitCharacters`): typing one of the
                // deliberately-selected language item's commit characters
                // accepts it, then the character types normally. The menu
                // alone never commits — only a chosen item, exactly the
                // Enter bar. Word candidates carry no commit characters.
                if self.completion_commit_character(ch) {
                    self.completion_accept_selected();
                }
                self.completion_typing(key);
                KeyDisposition::PassThrough
            }
            Key::Backspace => {
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

    /// 0069 D05 commit characters (LSP
    /// `completionItem.commitCharacters`): the typed character is one of
    /// the deliberately-selected language item's commit characters —
    /// accept it. Only a chosen item commits; the server-level
    /// `allCommitCharacters` capability is out of scope (absent in the
    /// servers strop supports today).
    pub(super) fn completion_commit_character(&self, ch: char) -> bool {
        let Some(session) = self.completion.session.as_ref() else {
            return false;
        };
        let Some((_, Candidate::Language { entry, .. })) = session.selected() else {
            return false;
        };
        entry
            .item
            .protocol()
            .commit_characters
            .as_ref()
            .is_some_and(|characters| {
                characters
                    .iter()
                    .any(|candidate| candidate.chars().eq(std::iter::once(ch)))
            })
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

    /// Tab cycling with a live preview (0059 §6.1): retire the previous
    /// preview, move the selection, then prepare the chosen candidate for a
    /// speculative apply through the same validated pipeline as acceptance.
    fn completion_preview_cycle(&mut self, direction: Direction) {
        self.completion_revert_preview();
        self.completion_navigate(direction);
        let Some(session) = &self.completion.session else {
            return;
        };
        if !session
            .query
            .context
            .still_owns(self, self.completion.settings_generation)
        {
            return;
        }
        // Collection excerpt targets stay acceptance-only in this amendment.
        let context = &session.query.context;
        if context
            .primary()
            .is_some_and(|primary| primary.source != context.scope.view)
        {
            return;
        }
        self.completion_preview_selected();
    }

    /// Retire one applied live preview: replay its exact inverse inside the
    /// same insert undo unit, restore the pre-preview selection shape and
    /// re-capture the owning context (buffer epochs are monotonic). A foreign
    /// interleaved edit refuses the revert instead of moving user data.
    pub(super) fn completion_revert_preview(&mut self) {
        let Some(active) = self
            .completion
            .session
            .as_mut()
            .and_then(|session| session.preview.take())
        else {
            return;
        };
        if self
            .docs
            .get(active.document)
            .is_none_or(|document| document.buf.revision() != active.revision)
        {
            debug_assert!(false, "completion preview revision guard");
            self.message = "completion preview diverged from its document".into();
            return;
        }
        let outcome = self.apply(
            active.document,
            active.revision,
            crate::editor::transact::ChangeSet {
                edits: active.inverse,
                undo_open: true,
            },
        );
        if let Err(error) = outcome {
            debug_assert!(false, "completion preview revert failed: {error}");
            self.message = format!("completion preview revert diverged: {error}");
            return;
        }
        let (request, invocation, scope, targets) = {
            let Some(session) = &self.completion.session else {
                return;
            };
            let targets: Vec<strop_core::selection::Selection> = session
                .query
                .context
                .targets
                .iter()
                .map(|target| target.selection)
                .collect();
            (
                session.query.request,
                session.query.context.invocation,
                session.query.context.scope.clone(),
                targets,
            )
        };
        let settings_generation = self.completion.settings_generation;
        let mut targets = targets.into_iter();
        if let Some(primary) = targets.next() {
            self.sels_mut()
                .stretch_primary(primary.anchor, primary.head);
        }
        self.sels_mut().set_extra_selections(targets);
        match super::context::capture(self, invocation, settings_generation, Some(&scope)) {
            Ok(fresh) => {
                if let Some(session) = &mut self.completion.session {
                    session.query = std::sync::Arc::new(super::model::CompletionQuery {
                        request,
                        context: std::sync::Arc::new(fresh),
                    });
                }
            }
            Err(_) => self.completion_close(CancelReason::Superseded),
        }
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
