use super::*;
use crate::editor::feedback::{
    FeedbackEvent, FeedbackKind, FeedbackOwner, FeedbackSeverity, TerminalFeedbackPhase,
    TypedAction,
};

/// Terminal operations carry their resource identity into the picker and are
/// checked again after the picker closes. None of these values contains child
/// input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalAction {
    SelectSessions,
    Stop {
        session: SessionId,
    },
    NewTerminal {
        document: DocumentId,
        session: SessionId,
    },
}

impl TerminalAction {
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::SelectSessions => "Select terminal session (Space b)",
            Self::Stop { .. } => "Stop terminal session",
            Self::NewTerminal { .. } => "New terminal (explicit launch)",
        }
    }
}

fn owner(session: SessionId, phase: &Phase) -> FeedbackOwner {
    FeedbackOwner::Terminal {
        session,
        // A SessionId is allocated once per admitted launch. Keeping the same
        // value as an explicit incarnation makes stale records unable to bind
        // to any later launch, even if document IDs are reused.
        incarnation: session.get(),
        phase: match phase {
            Phase::Starting | Phase::Running | Phase::Closing => TerminalFeedbackPhase::Live,
            Phase::Exited { .. } => TerminalFeedbackPhase::Exited,
            Phase::Failed(_) => TerminalFeedbackPhase::Failed,
        },
    }
}

fn record_terminal(editor: &mut Editor, event: FeedbackEvent) {
    let current = editor
        .feedback
        .unresolved()
        .any(|record| record.owner() == &event.owner && record.kind == event.kind);
    if !current {
        editor.record(event);
    }
}

impl Editor {
    pub(crate) fn terminal_owner_matches(
        &self,
        session: SessionId,
        incarnation: u64,
        phase: TerminalFeedbackPhase,
        document: DocumentId,
    ) -> bool {
        let Some(source) = self.terminal_document(document) else {
            return false;
        };
        let Some(entry) = self.terminals.entries.get(&source.session) else {
            return false;
        };
        source.session == session
            && incarnation == session.get()
            && (match phase {
                TerminalFeedbackPhase::Live => entry.phase.live(),
                TerminalFeedbackPhase::Exited => {
                    matches!(entry.phase, Phase::Exited { .. })
                }
                TerminalFeedbackPhase::Failed => matches!(entry.phase, Phase::Failed(_)),
            })
    }

    pub(crate) fn terminal_action_valid(&self, action: &TerminalAction) -> bool {
        match action {
            TerminalAction::SelectSessions => self
                .docs
                .iter()
                .any(|(document, _)| self.terminal_document(document).is_some()),
            TerminalAction::Stop { session } => self
                .terminals
                .entries
                .get(session)
                .is_some_and(|entry| entry.phase.live()),
            TerminalAction::NewTerminal { document, session } => {
                self.current() == *document
                    && self.terminal_document(*document).is_some_and(|source| {
                        source.session == *session
                            && self
                                .terminals
                                .entries
                                .get(session)
                                .is_some_and(|entry| !entry.phase.live())
                    })
            }
        }
    }

    pub(crate) fn invoke_terminal_action(&mut self, action: TerminalAction) {
        if !self.terminal_action_valid(&action) {
            self.message = "terminal action is stale; reopen actions for the live session".into();
            return;
        }
        match action {
            TerminalAction::SelectSessions => self.open_picker(strop_picker::Kind::Buffers),
            TerminalAction::Stop { session } => self.stop_terminal_session(session),
            TerminalAction::NewTerminal { session, .. } => {
                if let Some(location) = self
                    .terminals
                    .entries
                    .get(&session)
                    .map(|entry| entry.launch.clone())
                {
                    self.launch_terminal_at(location, "");
                }
            }
        }
    }

    pub(crate) fn terminal_context_actions(&self) -> Vec<TerminalAction> {
        let document = self.current();
        let Some(source) = self.terminal_document(document) else {
            return Vec::new();
        };
        let session = source.session;
        let Some(entry) = self.terminals.entries.get(&session) else {
            return Vec::new();
        };
        let mut actions = vec![TerminalAction::SelectSessions];
        if entry.phase.live() {
            actions.push(TerminalAction::Stop { session });
        } else {
            actions.push(TerminalAction::NewTerminal { document, session });
        }
        actions
    }

    pub(crate) fn terminal_action_title(&self, action: &TerminalAction) -> String {
        let session = match action {
            TerminalAction::Stop { session } | TerminalAction::NewTerminal { session, .. } => {
                Some(*session)
            }
            TerminalAction::SelectSessions => None,
        };
        session.map_or_else(
            || action.label().into(),
            |session| {
                let launch = self
                    .terminals
                    .entries
                    .get(&session)
                    .map(|entry| entry.launch.label())
                    .unwrap_or_else(|| "stale session".into());
                format!(
                    "{} — terminal #{} · {launch}",
                    action.label(),
                    session.get()
                )
            },
        )
    }

    /// Settle only records for this exact SessionId. All details are
    /// metadata; no terminal payload enters durable feedback.
    pub(super) fn sync_terminal_feedback(&mut self, document: DocumentId, denied: bool) {
        let Some((session, phase, launch)) = self.terminal_document(document).and_then(|source| {
            let entry = self.terminals.entries.get(&source.session)?;
            Some((source.session, entry.phase.clone(), entry.launch.label()))
        }) else {
            return;
        };

        if !phase.live() {
            let (severity, summary, detail) = match &phase {
                Phase::Exited {
                    code: Some(0),
                    signal: None,
                } => (
                    FeedbackSeverity::Info,
                    format!("terminal #{} exited 0", session.get()),
                    "Output retained; New terminal is always an explicit launch.".into(),
                ),
                Phase::Exited { code, signal } => (
                    FeedbackSeverity::Warning,
                    format!("terminal #{} ended", session.get()),
                    format!(
                        "Exit code {code:?}, signal {signal:?}; output retained at {launch}."
                    ),
                ),
                Phase::Failed(_) => (
                    FeedbackSeverity::Failure,
                    format!("terminal #{} failed", session.get()),
                    format!("Terminal process failed in {launch}; launch details retained without command or child payloads."),
                ),
                _ => unreachable!(),
            };
            record_terminal(
                self,
                FeedbackEvent {
                    severity,
                    owner: owner(session, &phase),
                    kind: FeedbackKind::TerminalProcess,
                    summary,
                    detail,
                    reference: None,
                    actions: vec![
                        TypedAction::Terminal(TerminalAction::NewTerminal { document, session }),
                        TypedAction::Terminal(TerminalAction::SelectSessions),
                        TypedAction::Dismiss,
                    ],
                },
            );
        }

        if denied {
            self.record(FeedbackEvent {
                severity: FeedbackSeverity::Failure,
                owner: owner(session, &phase),
                kind: FeedbackKind::TerminalHostControl,
                summary: format!("terminal #{} host control denied", session.get()),
                detail: format!("The child request was not performed. Launch namespace: {launch}."),
                reference: None,
                actions: vec![TypedAction::Dismiss],
            });
        }
    }

    pub(super) fn resolve_terminal_session_feedback(&mut self, session: SessionId) {
        for kind in [
            FeedbackKind::TerminalProcess,
            FeedbackKind::TerminalHostControl,
        ] {
            self.feedback.resolve_terminal(session, kind);
        }
    }
}
