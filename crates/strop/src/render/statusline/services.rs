//! Compact, owner-scoped language-service badges.

use ratatui::style::Style;
use ratatui::text::Span;
use strop_lsp::Severity;

use crate::editor::{Editor, LspPhase};
use crate::render::clip::width;
use crate::render::{muted, secondary, seed, severity_color};

pub(super) struct Services {
    badges: Vec<Badge>,
}

struct Badge {
    text: String,
    phase: LspPhase,
}

impl Services {
    pub(super) fn gather(editor: &Editor) -> Self {
        let mut badges: Vec<_> = editor
            .lsp_statuses()
            .map(|status| {
                let resource = status
                    .resource
                    .file_name()
                    .unwrap_or(status.resource.as_os_str())
                    .to_string_lossy();
                let scope = if status.target.is_remote()
                    || matches!(status.target, strop_workspace::Filesystem::Container(_))
                {
                    format!("{}:{}:{}", status.name, status.target.label(), resource)
                } else if resource.is_empty() {
                    status.name.to_owned()
                } else {
                    format!("{}@{}", status.name, resource)
                };
                Badge {
                    text: super::printable(format!("lsp {scope} {}", status.phase.label())),
                    phase: status.phase,
                }
            })
            .collect();
        // HashMap-backed discovery/refusal owners must not make the row
        // reorder between frames. Identical labels from separate scopes are
        // intentionally retained: they are distinct owned states.
        badges.sort_by(|a, b| {
            a.text
                .cmp(&b.text)
                .then(a.phase.label().cmp(b.phase.label()))
        });
        Self { badges }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.badges.is_empty()
    }

    pub(super) fn width(&self) -> usize {
        let labels = self
            .badges
            .iter()
            .map(|badge| width(&badge.text))
            .sum::<usize>();
        labels + self.badges.len().saturating_sub(1)
    }

    pub(super) fn extend_spans<'a>(&'a self, spans: &mut Vec<Span<'a>>) {
        for (index, badge) in self.badges.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            let color = match badge.phase {
                LspPhase::Starting => secondary(),
                LspPhase::Ready => muted(),
                LspPhase::Refused => seed(strop_core::theme::current().warning),
                LspPhase::Failed => severity_color(Severity::Error),
            };
            spans.push(Span::styled(
                badge.text.as_str(),
                Style::default().fg(color),
            ));
        }
    }

    pub(super) fn drop_ready(&mut self) {
        self.badges.retain(|badge| badge.phase != LspPhase::Ready);
    }

    pub(super) fn drop_starting(&mut self) {
        self.badges
            .retain(|badge| badge.phase != LspPhase::Starting);
    }

    /// Collapse outstanding problems to a bounded tail. The complete scoped
    /// list and reasons remain in :explain; the modeline keeps a non-color
    /// failure/refusal cue even when there is no room for their labels.
    pub(super) fn compact(&mut self) -> bool {
        let failed = self
            .badges
            .iter()
            .filter(|badge| badge.phase == LspPhase::Failed)
            .count();
        let refused = self
            .badges
            .iter()
            .filter(|badge| badge.phase == LspPhase::Refused)
            .count();
        let mut compact = Vec::with_capacity(usize::from(failed > 0) + usize::from(refused > 0));
        if failed > 0 {
            compact.push(Badge {
                text: if failed == 1 {
                    "LSP!".into()
                } else {
                    format!("LSP!{failed}")
                },
                phase: LspPhase::Failed,
            });
        }
        if refused > 0 {
            compact.push(Badge {
                text: if refused == 1 {
                    "LSP?".into()
                } else {
                    format!("LSP?{refused}")
                },
                phase: LspPhase::Refused,
            });
        }
        let changed = compact.len() != self.badges.len()
            || compact
                .iter()
                .zip(&self.badges)
                .any(|(new, old)| new.text != old.text);
        if changed {
            self.badges = compact;
        }
        changed
    }

    pub(super) fn problem_mark(&self) -> &'static str {
        if self
            .badges
            .iter()
            .any(|badge| badge.phase == LspPhase::Failed)
        {
            "LSP!"
        } else if self
            .badges
            .iter()
            .any(|badge| badge.phase == LspPhase::Refused)
        {
            "LSP?"
        } else {
            ""
        }
    }
}
