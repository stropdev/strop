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
            let _ = writeln!(text, "  readonly  yes — {}", self.readonly_explanation());
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
            // 0056 AR14: the winning layer with its origin, per knob.
            let layer = match self.config.knob_layer(knob.key) {
                crate::config::ConfigLayer::Default => "default".to_string(),
                crate::config::ConfigLayer::User => match self.config.user_layer_path() {
                    Some(path) => format!("user {}", path.display()),
                    None => "user config.toml".into(),
                },
            };
            let _ = writeln!(
                text,
                "  {} = {}  [{}]  — {}",
                knob.key, value, layer, knob.desc
            );
        }
        text.push_str(
            "  (user config.toml layers over embedded defaults; project layering is languages.toml under :trust)\n",
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

        // 0056 AR04: recovery tells the truth — policy, the durable
        // watermark, consent and failures; never a bare "enabled" flag.
        let status = self.recovery_status();
        text.push_str("\n[recovery]\n");
        if status.memory_only {
            text.push_str(
                "  policy      memory-only — drafts are NOT durable; nothing is persisted\n",
            );
        } else {
            text.push_str("  policy      automatic — dirty local documents and scratch drafts checkpoint to private state storage\n");
        }
        match status.durable_cohort {
            Some((cohort, ms)) => {
                let _ = writeln!(
                    text,
                    "  checkpoint  durable cohort {cohort} captured at {ms} ({} draft(s))",
                    status.durable_records
                );
            }
            None => text.push_str("  checkpoint  none completed this session\n"),
        }
        if status.in_flight || status.queued {
            text.push_str("  checkpoint  publication in progress — the guarantee is the last COMPLETED cohort\n");
        }
        let _ = writeln!(
            text,
            "  remote      {}",
            if status.consent_remote {
                "session consent granted — remote drafts persist"
            } else {
                "no consent — remote/sensitive drafts are not persisted (:recover consent remote)"
            }
        );
        if !status.over_bound.is_empty() {
            let _ = writeln!(
                text,
                "  over-limit  {} draft(s) exceed the 16 MiB capture limit — not durable",
                status.over_bound.len()
            );
        }
        if let Some(error) = &status.last_error {
            let _ = writeln!(text, "  last error  {error}");
        }

        // 0056 AR08: the effect/privacy classification actually in force
        // for the current document's target — from the typed classifier,
        // never a restated policy file.
        let policy = self.effect_policy();
        let target = super::privacy::target_of(&self.cur().source);
        text.push_str("\n[effects]\n");
        let _ = writeln!(text, "  target        {}", target.label());
        for family in super::privacy::EffectFamily::ALL {
            let class = super::privacy::classify(&policy, family, target);
            let capture = match class.capture {
                strop_trace::ContentPolicy::Full => "full content",
                strop_trace::ContentPolicy::Metadata => "metadata only",
            };
            let _ = writeln!(
                text,
                "  {:<13} {:<14} {}",
                family.label(),
                capture,
                class.reason
            );
        }
        let admitted = super::privacy::persistence_admitted(&policy, target);
        let _ = writeln!(
            text,
            "  persistence   {}",
            if admitted {
                "drafts admitted to private state storage".to_string()
            } else if target == super::privacy::EffectTarget::Ssh {
                "remote drafts not persisted without :recover consent remote".to_string()
            } else {
                "container bytes are never draft-persisted".to_string()
            }
        );
        if !strop_trace::enabled() {
            text.push_str("  (no trace active — capture policy applies when recording)\n");
        }

        let mut buffer = strop_core::Buffer::from_text(&text);
        buffer.name = Some("explain".into());
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from, like :help
        let _ = self.open_temporary_output(buffer);
    }
    /// The true source of the current buffer's readonly policy (0056
    /// AR14): the typed terminal owner first, then the recorded reason
    /// — never a generic hint.
    fn readonly_explanation(&self) -> &'static str {
        if self.terminal_document(self.current()).is_some() {
            return "terminal session — the child program owns the output; i enters child input";
        }
        match self.buf().readonly_reason {
            Some(strop_core::ReadonlyReason::Filesystem) => {
                "filesystem permissions report not writable — :set noro to edit anyway, :w! to force a write"
            }
            Some(strop_core::ReadonlyReason::Command) => {
                "set by :set ro / :view — :set noro restores editing"
            }
            Some(strop_core::ReadonlyReason::RemoteAuthority) => {
                "remote snapshot without write authority — :remote edit grants it"
            }
            Some(strop_core::ReadonlyReason::Container) => {
                "container file — container bytes have no local write path"
            }
            Some(strop_core::ReadonlyReason::GitSurface) => {
                "git surface — content is derived from history"
            }
            Some(strop_core::ReadonlyReason::Output) => "transient output view — not a source",
            Some(strop_core::ReadonlyReason::DirectoryListing) => {
                "directory listing — :fs edit opens filename drafts"
            }
            Some(strop_core::ReadonlyReason::DirectoryOperation) => {
                "directory operation in flight — editing resumes when it lands"
            }
            Some(strop_core::ReadonlyReason::CollectionProjection) => {
                "collection projection failed — this stale view refuses edits"
            }
            Some(strop_core::ReadonlyReason::RecoveryCheckpoint) => {
                "recovery checkpoint surface — :recover restore N opens a checked draft"
            }
            Some(strop_core::ReadonlyReason::OutsideWorkspace) => {
                "outside the workspace root — :set noro to edit"
            }
            None => "set directly — :set noro restores editing",
        }
    }
}

/// A refusal with its actionable reason — the install hint or the trust
/// command, not just a label.
pub(crate) fn explain_decision(decision: &AttachDecision) -> String {
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
    /// 0056 AR14: a file whose permissions report not writable opens
    /// readonly, and :explain names the filesystem as the source.
    #[test]
    fn explain_names_filesystem_readonly_source() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        std::fs::write(&path, "locked\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let buffer = Buffer::open(&path).unwrap();
        assert_eq!(
            buffer.readonly_reason,
            Some(strop_core::ReadonlyReason::Filesystem)
        );
        let mut e = Editor::new(buffer);
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(
            text.contains("filesystem permissions report not writable"),
            "{text}"
        );
        assert!(!text.contains(":remote edit grants"), "{text}");
    }

    /// 0056 AR14: a remote snapshot names its authority source; AR08:
    /// the effects section follows the document's ssh target.
    #[test]
    fn explain_names_remote_readonly_source_and_ssh_target() {
        use crate::editor::document::RemoteDocument;
        let file = strop_workspace::RemoteFile::parse("ssh://fixture/work/file.txt").unwrap();
        let selection = strop_remote::ReadSelection::Full;
        let mut e = Editor::new(Buffer::from_text("local\n"));
        let document = e
            .docs
            .try_insert(crate::editor::Document::remote(
                Buffer::from_text("before\n"),
                RemoteDocument {
                    file,
                    window: strop_remote::RemoteWindow::resolve(
                        &selection,
                        strop_remote::RemoteSize::new(7),
                    ),
                    selection,
                    connection: None,
                    return_to: None,
                    write: None,
                },
            ))
            .unwrap();
        e.switch_to(document);
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(
            text.contains("remote snapshot without write authority — :remote edit grants it"),
            "{text}"
        );
        assert!(text.contains("target        ssh"), "{text}");
        assert!(
            text.contains("remote drafts not persisted without :recover consent remote"),
            "{text}"
        );
    }

    /// 0056 AR14: `:set ro` is its own source — not a remote hint.
    #[test]
    fn explain_names_command_readonly_source() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":set ro\r");
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(text.contains("set by :set ro / :view"), "{text}");
        assert!(!text.contains(":remote edit grants"), "{text}");
    }

    /// 0056 AR14: the recovery checkpoint surface names itself.
    #[test]
    fn explain_names_recovery_surface_readonly_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
        e.feed_text(":recover\r");
        assert_eq!(
            e.buf().readonly_reason,
            Some(strop_core::ReadonlyReason::RecoveryCheckpoint)
        );
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(text.contains("recovery checkpoint surface"), "{text}");
    }

    /// 0056 AR14: a terminal session names the child program as the
    /// owner of its output — the typed terminal source, not the generic
    /// output reason the document was built with.
    #[test]
    fn explain_names_terminal_readonly_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new_in(Buffer::from_text("origin"), dir.path().to_path_buf());
        e.terminal_fixture(
            &[("safe", strop_terminal::model::Style::default())],
            strop_terminal::model::Phase::Exited {
                code: Some(0),
                signal: None,
            },
        );
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(
            text.contains("terminal session — the child program owns the output"),
            "{text}"
        );
    }

    /// 0056 AR14: config provenance shows the actual winning layer per
    /// knob — the user file with its path where it won, default else.
    #[test]
    fn explain_shows_config_provenance_layers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "tab_size = 8\n").unwrap();
        let (config, error) = crate::config::Config::load_from(&path);
        assert!(error.is_none());
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.config = config;
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(
            text.contains(&format!("tab_size = 8  [user {}]", path.display())),
            "{text}"
        );
        assert!(text.contains("indent_guides = true  [default]"), "{text}");
        assert!(text.contains("cursor_fade = true  [default]"), "{text}");
    }

    /// 0056 AR08: the effects section renders the classifier's actual
    /// decisions for the local target with no opt-ins granted.
    #[test]
    fn explain_lists_effect_classification() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_explain();
        let text = e.buf().text().to_string();
        assert!(text.contains("[effects]"), "{text}");
        assert!(text.contains("target        local"), "{text}");
        assert!(
            text.contains("clipboard     metadata only  clipboard payloads never enter"),
            "{text}"
        );
        assert!(
            text.contains("process       metadata only  command lines and environment"),
            "{text}"
        );
        assert!(
            text.contains("persistence   drafts admitted to private state storage"),
            "{text}"
        );
        assert!(text.contains("no trace active"), "{text}");
    }
}
