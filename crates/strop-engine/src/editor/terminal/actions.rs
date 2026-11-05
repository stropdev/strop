use super::*;
use crate::editor::feedback::{
    FeedbackEvent, FeedbackKind, FeedbackOwner, FeedbackSeverity, TerminalFeedbackPhase,
    TypedAction,
};

/// Terminal operations carry their resource identity into the picker and are
/// checked again after the picker closes. None of these values contains child
/// input or paste contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalAction {
    Refresh {
        document: DocumentId,
        session: SessionId,
    },
    ReviewPaste {
        session: SessionId,
        ticket: u64,
    },
    DecidePaste {
        session: SessionId,
        ticket: u64,
        accept: bool,
    },
    SelectSessions,
    Stop {
        session: SessionId,
    },
    NewTerminal {
        document: DocumentId,
        session: SessionId,
    },
    Focus {
        session: SessionId,
    },
    QuitReturn,
}

impl TerminalAction {
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::Refresh { .. } => "Refresh pinned terminal output",
            Self::ReviewPaste { .. } => "Review held paste (content redacted)",
            Self::DecidePaste { accept: true, .. } => "Send held paste",
            Self::DecidePaste { accept: false, .. } => "Discard held paste",
            Self::SelectSessions => "Select terminal session (Space b)",
            Self::Stop { .. } => "Stop terminal session",
            Self::NewTerminal { .. } => "New terminal (explicit launch)",
            Self::Focus { .. } => "Focus terminal session",
            Self::QuitReturn => "Return without stopping terminal sessions",
        }
    }
}

fn owner(session: SessionId, phase: &Phase, ticket: Option<u64>) -> FeedbackOwner {
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
        ticket,
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
        ticket: Option<u64>,
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
            && ticket.is_none_or(|ticket| {
                entry
                    .paste
                    .as_ref()
                    .is_some_and(|paste| paste.ticket == ticket)
            })
    }

    pub(crate) fn terminal_action_valid(&self, action: &TerminalAction) -> bool {
        match action {
            TerminalAction::Refresh { document, session } => {
                self.current() == *document
                    && self.terminal_document(*document).is_some_and(|source| {
                        source.session == *session
                            && self
                                .terminals
                                .entries
                                .get(session)
                                .is_some_and(|entry| entry.live.is_some())
                    })
                    && !self.terminal_input_active()
            }
            TerminalAction::ReviewPaste { session, ticket } => self
                .terminal_document(self.current())
                .is_some_and(|source| {
                    source.session == *session
                        && self.terminals.entries.get(session).is_some_and(|entry| {
                            entry.phase.live()
                                && entry
                                    .paste
                                    .as_ref()
                                    .is_some_and(|paste| paste.ticket == *ticket)
                        })
                }),
            TerminalAction::DecidePaste {
                session, ticket, ..
            } => self
                .terminal_document(self.current())
                .is_some_and(|source| {
                    source.session == *session
                        && self.terminals.entries.get(session).is_some_and(|entry| {
                            matches!(entry.phase, Phase::Running)
                                && entry
                                    .paste
                                    .as_ref()
                                    .is_some_and(|paste| paste.ticket == *ticket)
                        })
                }),
            TerminalAction::SelectSessions => self
                .docs
                .iter()
                .any(|(document, _)| self.terminal_document(document).is_some()),
            TerminalAction::Stop { session } | TerminalAction::Focus { session } => self
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
            TerminalAction::QuitReturn => true,
        }
    }

    pub(crate) fn invoke_terminal_action(&mut self, action: TerminalAction) {
        if !self.terminal_action_valid(&action) {
            self.message = "terminal action is stale; reopen actions for the live session".into();
            return;
        }
        match action {
            TerminalAction::Refresh { .. } => self.refresh_terminal(),
            TerminalAction::ReviewPaste { session, ticket } => {
                self.terminal_paste_decision_for(session, ticket, None);
            }
            TerminalAction::DecidePaste {
                session,
                ticket,
                accept,
            } => self.terminal_paste_decision_for(session, ticket, Some(accept)),
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
            TerminalAction::Focus { session } => {
                if let Some(document) = self
                    .terminals
                    .entries
                    .get(&session)
                    .map(|entry| entry.document)
                {
                    self.switch_to(document);
                    self.message =
                        format!("focused terminal #{}; process still running", session.get());
                }
            }
            TerminalAction::QuitReturn => {
                self.message = "quit cancelled; terminal sessions remain running".into();
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
        let mut actions = Vec::new();
        if !self.terminal_input_active() && entry.live.is_some() {
            actions.push(TerminalAction::Refresh { document, session });
        }
        if let Some(ticket) = entry.paste.as_ref().map(|paste| paste.ticket) {
            actions.push(TerminalAction::ReviewPaste { session, ticket });
            if matches!(entry.phase, Phase::Running) {
                actions.extend([
                    TerminalAction::DecidePaste {
                        session,
                        ticket,
                        accept: true,
                    },
                    TerminalAction::DecidePaste {
                        session,
                        ticket,
                        accept: false,
                    },
                ]);
            }
        }
        actions.push(TerminalAction::SelectSessions);
        if entry.phase.live() {
            actions.push(TerminalAction::Stop { session });
        } else {
            actions.push(TerminalAction::NewTerminal { document, session });
        }
        actions
    }

    pub(crate) fn terminal_quit_actions(&self) -> Vec<TerminalAction> {
        let mut sessions: Vec<_> = self
            .terminals
            .entries
            .values()
            .filter(|entry| entry.phase.live())
            .map(|entry| {
                let Some(document) = self.terminal_document(entry.document) else {
                    panic!("terminal entry retains its document");
                };
                document.session
            })
            .collect();
        sessions.sort_unstable_by_key(|session| session.get());
        let mut actions = Vec::new();
        for session in sessions {
            actions.push(TerminalAction::Focus { session });
            actions.push(TerminalAction::Stop { session });
        }
        actions.push(TerminalAction::QuitReturn);
        actions
    }

    pub(crate) fn terminal_action_title(&self, action: &TerminalAction) -> String {
        let session = match action {
            TerminalAction::Refresh { session, .. }
            | TerminalAction::ReviewPaste { session, .. }
            | TerminalAction::DecidePaste { session, .. }
            | TerminalAction::Stop { session }
            | TerminalAction::NewTerminal { session, .. }
            | TerminalAction::Focus { session } => Some(*session),
            TerminalAction::SelectSessions | TerminalAction::QuitReturn => None,
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

    /// Replace each live decision in place, and settle only records for this
    /// exact SessionId. All details are metadata; no terminal or paste payload
    /// enters durable feedback.
    pub(super) fn sync_terminal_feedback(&mut self, document: DocumentId, denied: bool) {
        let Some((session, phase, paste, launch, has_output, inspecting)) =
            self.terminal_document(document).and_then(|source| {
                let entry = self.terminals.entries.get(&source.session)?;
                Some((
                    source.session,
                    entry.phase.clone(),
                    entry.paste.clone(),
                    entry.launch.label(),
                    self.terminal_has_new_output(document),
                    self.panes
                        .iter()
                        .any(|pane| pane.doc == document && !pane.terminal_input),
                ))
            })
        else {
            return;
        };

        if phase.live() && inspecting && has_output {
            record_terminal(self, FeedbackEvent {
                severity: FeedbackSeverity::Info,
                owner: owner(session, &phase, None),
                kind: FeedbackKind::TerminalOutput,
                summary: format!("terminal #{session} has new output", session = session.get()),
                detail: format!(
                    "Pinned snapshot for terminal #{} at {launch}; refresh only on explicit request.",
                    session.get()
                ),
                reference: None,
                actions: vec![
                    TypedAction::Terminal(TerminalAction::Refresh { document, session }),
                    TypedAction::Dismiss,
                ],
            });
        } else {
            self.feedback
                .resolve_terminal(session, FeedbackKind::TerminalOutput, None);
        }

        self.feedback.resolve_terminal(
            session,
            FeedbackKind::TerminalPaste,
            paste.as_ref().map(|paste| paste.ticket),
        );
        if let Some(paste) = paste.filter(|_| phase.live()) {
            let ticket = paste.ticket;
            record_terminal(
                self,
                FeedbackEvent {
                    severity: FeedbackSeverity::Warning,
                    owner: owner(session, &phase, Some(ticket)),
                    kind: FeedbackKind::TerminalPaste,
                    summary: format!("terminal #{} paste held", session.get()),
                    detail: format!("{} · {launch}", paste.review()),
                    reference: None,
                    actions: vec![
                        TypedAction::Terminal(TerminalAction::ReviewPaste { session, ticket }),
                        TypedAction::Terminal(TerminalAction::DecidePaste {
                            session,
                            ticket,
                            accept: true,
                        }),
                        TypedAction::Terminal(TerminalAction::DecidePaste {
                            session,
                            ticket,
                            accept: false,
                        }),
                        TypedAction::Dismiss,
                    ],
                },
            );
        }

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
                    owner: owner(session, &phase, None),
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
                owner: owner(session, &phase, None),
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
            FeedbackKind::TerminalOutput,
            FeedbackKind::TerminalPaste,
            FeedbackKind::TerminalProcess,
            FeedbackKind::TerminalHostControl,
        ] {
            self.feedback.resolve_terminal(session, kind, None);
        }
    }
}
