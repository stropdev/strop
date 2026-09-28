//! Readonly presentation shared by terminal/headless/GUI consumers. Providers
//! prepare bounded data; painting never launches work or mutates ownership.
use super::model::{CompletionPayload, PreparedDocumentation};
use super::session::{Acceptance, Candidate, ProviderState, ResolveState, Session};
use crate::editor::Editor;
use strop_core::worker::Outcome;
use strop_lsp::completion::CompletionItemKind as Kind;

pub struct CompletionMenu<'a> {
    session: &'a Session,
    prefix: ropey::RopeSlice<'a>,
}
pub struct CompletionRow<'a> {
    pub label: &'a str,
    pub kind: &'static str,
    pub source: &'static str,
    pub detail: &'a str,
    pub unsupported: Option<&'static str>,
}
pub struct CompletionDocumentation<'a> {
    pub text: &'a str,
    pub spans: &'a [strop_syntax::Span],
    pub truncated: bool,
}
#[derive(Clone, Copy)]
pub struct CompletionProviderStatus<'a> {
    pub source: &'static str,
    pub state: &'static str,
    pub explanation: &'a str,
}

impl Editor {
    pub fn completion_menu(&self) -> Option<CompletionMenu<'_>> {
        let session = self.completion.session.as_ref()?;
        if !session
            .query
            .context
            .still_owns(self, self.completion.settings_generation)
            || session.language().is_some_and(|language| {
                !language
                    .owner
                    .owns(self, &session.query.context, language.origin.stamp)
            })
            || (!session.query.context.invocation.manual() && session.count() == 0)
        {
            return None;
        }
        let primary = session.query.context.primary()?;
        let source = self.docs.get(primary.source)?;
        let prefix = source
            .buf
            .text()
            .get_byte_slice(primary.start..primary.caret)?;
        Some(CompletionMenu { session, prefix })
    }

    /// Bounded logical presentation for semantic UI and forensic observations.
    /// Metadata capture deliberately excludes labels, docs, edits and opaque data.
    pub(crate) fn completion_snapshot(&self, content: bool) -> serde_json::Value {
        use super::session::WorkerLifecycle;
        use serde_json::json;
        let session = self.completion.session.as_ref();
        let menu = self.completion_menu();
        let providers = session.map(|session| {
            [
                provider("lsp", &session.language),
                provider("buf", &session.words),
            ]
            .map(|provider| {
                json!({
                    "source": provider.source, "state": provider.state,
                    "explanation": content.then(|| bounded_label(provider.explanation)),
                })
            })
        });
        let work = session.and_then(|session| match &session.words {
            ProviderState::Settled(delivery) => match &delivery.payload {
                CompletionPayload::Words { work, .. } => Some(work),
                _ => None,
            },
            _ => None,
        });
        let visible = menu.as_ref().map(|menu| {
            let start = menu.selected().unwrap_or(0).saturating_sub(5);
            let rows = content.then(|| {
                (start..menu.len().min(start + 6))
                    .filter_map(|index| {
                        menu.row(index).map(|row| {
                            json!({
                                "index": index, "label": bounded_label(row.label),
                                "label_truncated": row.label.len() > 512,
                                "kind": row.kind, "source": row.source,
                                "detail": bounded_label(row.detail),
                                "detail_truncated": row.detail.len() > 512,
                                "unsupported": row.unsupported,
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            });
            json!({
                "pane": menu.pane(), "anchor_byte": menu.anchor_byte(),
                "count": menu.len(), "selected": menu.selected(),
                "first_row": start, "rows": rows,
                "incomplete": menu.incomplete(), "limited": menu.limited(),
                "action": menu.action_status(),
                "documentation": content.then(|| menu.documentation().map(|docs| json!({
                    "text": docs.text, "spans": docs.spans, "truncated": docs.truncated,
                }))).flatten(),
            })
        });
        json!({
            "enabled": self.config.completion.enabled,
            "auto_popup": self.config.completion.auto_popup,
            "settings_generation": self.completion.settings_generation,
            "worker": match self.completion.worker {
                WorkerLifecycle::Idle => "idle",
                WorkerLifecycle::Failed(_) => "failed",
                WorkerLifecycle::Running { .. } => "running",
                WorkerLifecycle::Stopping { .. } => "retiring",
            },
            "query": session.map(|session| session.query.request),
            "source": session.and_then(|session| session.query.context.primary())
                .map(|primary| json!({"document": primary.source, "revision": primary.revision})),
            "invocation": session.map(|session| session.query.context.invocation),
            "providers": providers, "menu": visible,
            "publication": self.completion.metrics, "work": work,
        })
    }
}

impl<'a> CompletionMenu<'a> {
    pub fn anchor_byte(&self) -> usize {
        self.session
            .query
            .context
            .primary()
            .map_or(0, |target| target.view_start)
    }
    pub fn pane(&self) -> usize {
        self.session.query.context.scope.pane
    }
    pub fn prefix(&self) -> ropey::RopeSlice<'a> {
        self.prefix
    }
    pub fn len(&self) -> usize {
        self.session.count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn selected(&self) -> Option<usize> {
        self.session.selected().map(|(index, _)| index)
    }
    pub fn row(&self, index: usize) -> Option<CompletionRow<'a>> {
        let candidate = self.session.candidate(index)?;
        Some(match candidate {
            Candidate::Word(word) => CompletionRow {
                label: &word.text,
                kind: "word",
                source: "buf",
                detail: "",
                unsupported: None,
            },
            Candidate::Language { entry, .. } => {
                let item = &entry.item.protocol();
                let unsupported =
                    super::acceptance::unsupported_item(&entry.item).map(|refusal| match refusal {
                        super::acceptance::AcceptanceRefusal::Snippet => "snippet unsupported",
                        super::acceptance::AcceptanceRefusal::WhitespaceMode => {
                            "whitespace adjustment unsupported"
                        }
                        super::acceptance::AcceptanceRefusal::Command => {
                            "server command unsupported"
                        }
                        _ => unreachable!(
                            "item support only classifies represented protocol effects"
                        ),
                    });
                CompletionRow {
                    label: entry.item.label(),
                    kind: kind(item.kind),
                    source: "lsp",
                    detail: item.detail.as_deref().unwrap_or(""),
                    unsupported,
                }
            }
        })
    }
    pub fn providers(&self) -> [CompletionProviderStatus<'a>; 2] {
        [
            provider("lsp", &self.session.language),
            provider("buf", &self.session.words),
        ]
    }
    pub fn incomplete(&self) -> bool {
        self.session
            .language()
            .is_some_and(|language| language.items.server_incomplete)
    }
    pub fn limited(&self) -> bool {
        self.session
            .language()
            .is_some_and(|language| language.items.client_truncated)
            || self.session.words().is_some_and(|words| {
                words.truncated || words.coverage == super::words::IndexCoverage::Limited
            })
    }
    pub fn action_status(&self) -> Option<&'static str> {
        match &self.session.acceptance {
            Acceptance::Resolving { .. } => return Some("resolving required edits"),
            Acceptance::Preparing { .. } => return Some("checking completion edits"),
            Acceptance::WaitingCapacity { candidate, work } => {
                debug_assert!(
                    self.session
                        .selected()
                        .is_some_and(|(_, selected)| selected.matches(candidate))
                        && self.session.preparation == Some(*work)
                );
                return Some("waiting for edit preparation capacity");
            }
            Acceptance::Idle => {}
        }
        match &self.session.resolve {
            ResolveState::Pending(_) => Some("resolving selected item"),
            ResolveState::WaitingCapacity => Some("selected item: server busy"),
            ResolveState::Failed(reason) => Some(match reason {
                strop_lsp::RequestRefusal::Unsupported => "selected item cannot resolve",
                strop_lsp::RequestRefusal::Oversized => "selected item exceeds the resolve bound",
                _ => "selected item resolve unavailable",
            }),
            ResolveState::Complete(delivery) => match &delivery.payload {
                CompletionPayload::Resolved { reply, .. }
                    if matches!(reply.outcome, Outcome::Failed { .. }) =>
                {
                    Some("selected item resolve failed")
                }
                _ => None,
            },
            ResolveState::Idle => None,
        }
    }
    pub fn documentation(&self) -> Option<CompletionDocumentation<'a>> {
        let delivery = self.session.documentation.as_ref()?;
        let CompletionPayload::Documentation {
            outcome:
                Outcome::Success(PreparedDocumentation {
                    text,
                    spans,
                    truncated,
                }),
            ..
        } = &delivery.payload
        else {
            return None;
        };
        if text.is_empty() {
            return None;
        }
        Some(CompletionDocumentation {
            text,
            spans,
            truncated: *truncated,
        })
    }
}

fn provider<'a>(source: &'static str, provider: &'a ProviderState) -> CompletionProviderStatus<'a> {
    let (state, explanation) = match provider {
        ProviderState::Off => ("off", "not requested"),
        ProviderState::Pending => ("pending", "working"),
        ProviderState::WaitingCapacity => ("busy", "waiting for server capacity"),
        ProviderState::Initializing => ("pending", "language service starting"),
        ProviderState::Unavailable(reason) => ("unavailable", *reason),
        ProviderState::Failed(failure) => ("failed", failure.message.as_str()),
        ProviderState::Settled(delivery) => match &delivery.payload {
            CompletionPayload::Words { outcome, .. } => match outcome {
                Outcome::Success(words) if words.items.is_empty() => {
                    ("empty", "no matching source words")
                }
                Outcome::Success(_) => ("ready", "current source words"),
                Outcome::Failed { failure, .. } => ("failed", failure.message.as_str()),
                Outcome::Cancelled(_) => ("cancelled", "query cancelled"),
            },
            CompletionPayload::Language(outcome) => match outcome {
                Outcome::Success(language) if language.items.items.is_empty() => {
                    ("empty", "no language completions")
                }
                Outcome::Success(_) => ("ready", "language service"),
                Outcome::Failed { failure, .. } => ("failed", failure.message.as_str()),
                Outcome::Cancelled(_) => ("cancelled", "query cancelled"),
            },
            _ => unreachable!("provider state contains its own typed result"),
        },
    };
    CompletionProviderStatus {
        source,
        state,
        explanation,
    }
}

fn bounded_label(text: &str) -> &str {
    &text[..text.floor_char_boundary(text.len().min(512))]
}

fn kind(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::METHOD) => "meth",
        Some(Kind::FUNCTION) => "func",
        Some(Kind::CONSTRUCTOR) => "ctor",
        Some(Kind::FIELD) => "field",
        Some(Kind::VARIABLE) => "var",
        Some(Kind::CLASS) => "class",
        Some(Kind::INTERFACE) => "iface",
        Some(Kind::MODULE) => "mod",
        Some(Kind::PROPERTY) => "prop",
        Some(Kind::UNIT) => "unit",
        Some(Kind::VALUE) => "value",
        Some(Kind::ENUM) => "enum",
        Some(Kind::KEYWORD) => "key",
        Some(Kind::SNIPPET) => "snip",
        Some(Kind::COLOR) => "color",
        Some(Kind::FILE) => "file",
        Some(Kind::REFERENCE) => "ref",
        Some(Kind::FOLDER) => "dir",
        Some(Kind::ENUM_MEMBER) => "enum",
        Some(Kind::CONSTANT) => "const",
        Some(Kind::STRUCT) => "struct",
        Some(Kind::EVENT) => "event",
        Some(Kind::OPERATOR) => "op",
        Some(Kind::TYPE_PARAMETER) => "type",
        _ => "text",
    }
}
