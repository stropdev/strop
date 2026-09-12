//! `:explain` — why things are the way they are (0042 slice 2 follow-on,
//! handoff "explain editor decisions"). A real readonly buffer (0001 §4):
//! `/` searches it, motions walk it, `q` closes it. Renders from the
//! editor's actual decision records — the workspace registry, attach
//! records and live config — never inferred from logs.

use std::fmt::Write;

use super::lsp::attach::AttachDecision;
use super::Editor;

impl Editor {
    /// Open the explain buffer: workspace bindings, the current document's
    /// routing, language-server readiness and refusals with their reasons,
    /// and effective configuration.
    pub(crate) fn open_explain(&mut self) {
        let mut text = String::from("strop explain — why things are the way they are\n");

        text.push_str("\n[workspaces]\n");
        for (id, context) in self.workspaces.iter() {
            let root = context
                .root
                .as_ref()
                .map(|root| root.display().to_string())
                .unwrap_or_else(|| "no project root".into());
            let _ = writeln!(
                text,
                "  {} #{}  {}  (incarnation {})",
                context.filesystem.label(),
                id.index(),
                root,
                context.incarnation
            );
        }

        text.push_str("\n[current document]\n");
        match self.lsp_current_doc_path() {
            Some(resource) => {
                let _ = writeln!(text, "  resource  {}", resource.label());
            }
            None => text.push_str("  resource  (scratch — no filesystem identity)\n"),
        }
        let _ = writeln!(
            text,
            "  state     {}, revision {}",
            if self.buf().dirty { "dirty" } else { "clean" },
            self.buf().revision().get()
        );
        if self.buf().readonly {
            text.push_str(
                "  readonly  yes — :remote edit grants write authority on remote files\n",
            );
        }
        match self.lsp_state.bindings.get(&self.current()) {
            Some(binding) => {
                let _ = writeln!(
                    text,
                    "  lsp       bound to server {} (root {}, on {})",
                    binding.server.get(),
                    binding.root.display(),
                    binding.target.label()
                );
            }
            None => text.push_str("  lsp       not bound\n"),
        }

        text.push_str("\n[language servers]\n");
        if self.lsp_servers.is_empty()
            && self.lsp_state.attach.pending.is_empty()
            && self.lsp_state.attach.refused.is_empty()
        {
            text.push_str("  none attached or attempted\n");
        }
        for server in &self.lsp_servers {
            let _ = writeln!(
                text,
                "  server {}  {}",
                server.id.get(),
                if server.ready { "ready" } else { "starting" }
            );
        }
        for (key, ticket) in &self.lsp_state.attach.pending {
            let _ = writeln!(
                text,
                "  {} on {}  attaching (request {})",
                key.language,
                key.target.label(),
                ticket.get()
            );
        }
        for (key, decision) in &self.lsp_state.attach.refused {
            let _ = writeln!(
                text,
                "  {} on {}  refused: {}",
                key.language,
                key.target.label(),
                explain_decision(decision)
            );
        }

        text.push_str("\n[configuration]\n");
        for knob in crate::config::KNOBS {
            // 0051 R10: real values from the one typed access path —
            // a knob that resolves to nothing is a test failure, never
            // a "?" in front of the user.
            let Some(value) = self.config.knob_value(knob.key) else {
                continue;
            };
            let _ = writeln!(text, "  {} = {}  — {}", knob.key, value, knob.desc);
        }
        text.push_str(
            "  (config.toml layers over embedded defaults; :trust gates project layers)\n",
        );

        // 0051 R08: the current document's effective indent with its
        // per-side provenance, the override state, and what detection
        // concluded (or why it stayed Unknown).
        let doc = self.cur();
        let indent = doc.indent;
        text.push_str("\n[indentation]\n");
        let _ = writeln!(
            text,
            "  effective: {} (style {}, width {})",
            indent.label(),
            indent.style_source.label(),
            indent.width_source.label(),
        );
        match (doc.indent_override.style, doc.indent_override.width) {
            (None, None) => text.push_str("  override: none (:tab-size / :indent-style set one)\n"),
            (style, width) => {
                let mut parts = Vec::new();
                if let Some(style) = style {
                    parts.push(format!("style {style:?}"));
                }
                if let Some(width) = width {
                    parts.push(format!("width {width}"));
                }
                let _ = writeln!(text, "  override: {}", parts.join(", "));
            }
        }
        match doc.detection {
            None => text.push_str("  detection: disabled (indent_detect = false)\n"),
            Some(crate::editor::document::Detection::Unknown(reason)) => {
                let _ = writeln!(
                    text,
                    "  detection: unknown — {} (configured fallback applies)",
                    reason.reason()
                );
            }
            Some(crate::editor::document::Detection::Tabs {
                evidence,
                confidence,
            }) => {
                let _ = writeln!(
                    text,
                    "  detection: tabs — {} confidence ({evidence} evidence lines)",
                    confidence.label()
                );
            }
            Some(crate::editor::document::Detection::Spaces {
                width,
                evidence,
                confidence,
            }) => {
                let _ = writeln!(
                    text,
                    "  detection: spaces, width {width} — {} confidence ({evidence} evidence lines)",
                    confidence.label()
                );
            }
        }

        let mut buffer = strop_core::Buffer::from_text(&text);
        buffer.name = Some("explain".into());
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from, like :help
        self.open_temporary_output(buffer);
    }
}

/// A refusal with its actionable reason — the install hint or the trust
/// command, not just a label.
fn explain_decision(decision: &AttachDecision) -> String {
    match decision {
        AttachDecision::TrustRequired { command } => {
            format!("project config wants `{command}` — :trust to allow")
        }
        AttachDecision::TrustError { error } => format!("trust check failed: {error}"),
        AttachDecision::NotExecutable {
            command, reason, ..
        } => {
            format!("`{command}` not executable: {reason}")
        }
        AttachDecision::SpawnFailed { reason } => format!("spawn failed: {reason}"),
        AttachDecision::RemoteIo { reason } => format!("remote discovery failed: {reason}"),
        other => other.label().replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn explain_lists_workspaces_and_config() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":explain\r");
        let text = e.buf().text().to_string();
        assert!(text.contains("[workspaces]"), "{text}");
        assert!(text.contains("local"), "{text}");
        assert!(text.contains("tab_size = 4"), "{text}");
        assert!(text.contains("lsp       not bound"), "{text}");
        assert!(e.buf().readonly, "explain is a real readonly buffer");
        e.feed_text("/[language servers]\r");
        assert!(e.head() > 0, "searchable like any buffer");
    }
    /// 0051 R08: the indentation section shows the effective setting,
    /// per-side provenance, the override, and detection's confidence —
    /// and the once-placeholder knobs show real values.
    #[test]
    fn explain_shows_indent_provenance_and_detection() {
        let mut e = Editor::new(Buffer::from_text(
            "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
        ));
        e.reresolve_indents();
        e.tab_size_command("8");
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(text.contains("[indentation]"), "{text}");
        assert!(text.contains("effective: Spaces:8"), "{text}");
        assert!(text.contains("style detected"), "{text}");
        assert!(text.contains("width manual"), "{text}");
        assert!(text.contains("override: width 8"), "{text}");
        assert!(text.contains("detection: spaces, width 2"), "{text}");
        assert!(text.contains("high confidence"), "{text}");
        assert!(text.contains("indent_style = spaces"), "{text}");
        assert!(text.contains("indent_detect = true"), "{text}");
        assert!(text.contains("auto_format = true"), "{text}");
        assert!(text.contains("search_show_hidden = true"), "{text}");
        assert!(text.contains("search_respect_ignore = true"), "{text}");
        assert!(!text.contains("= ?"), "no placeholder values: {text}");
    }
}
