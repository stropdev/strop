//! Bounded warm-up of eligible unopened projects (0063 §2): the
//! workspace-symbols source reports marker subprojects; each may start
//! its language's server without opening any document. At most four
//! warm attaches are in flight; the rest wait for a completion.
use super::super::super::picker::ProjectStatus;
use super::super::*;
use super::args_key;

impl Editor {
    /// Bounded warm-up of eligible unopened projects (0063 §2): the
    /// workspace-symbols source reports marker subprojects; each may
    /// start its language's server without opening any document. Warm
    /// servers are reused by the merge tier; servers serving active
    /// documents are never touched (attachments dedup by root).
    pub(crate) fn lsp_warm_scope_projects(&mut self, projects: Vec<(PathBuf, String)>) {
        if !self.lsp_state.attach.enabled {
            return;
        }
        if self
            .picker
            .as_ref()
            .is_none_or(|glue| glue.picker.kind != strop_picker::Kind::WorkspaceSymbols)
        {
            return;
        }
        for project in projects {
            if !self.lsp_state.attach.warm_queue.contains(&project) {
                self.lsp_state.attach.warm_queue.push_back(project);
            }
        }
        self.lsp_drain_warm_queue();
    }

    /// Keep at most four warm attaches in flight; the rest wait for a
    /// completion. Never every installed server at once.
    fn lsp_drain_warm_queue(&mut self) {
        const WARM_ATTACH_LIMIT: usize = 4;
        // WarmBounded (0063 §6.6): the verified kernel's slot decision.
        while strop_core::searchguard::warm_slot_free(
            self.lsp_state.attach.pending.len(),
            WARM_ATTACH_LIMIT,
        ) {
            let Some((root, marker)) = self.lsp_state.attach.warm_queue.pop_front() else {
                break;
            };
            // A closed picker stops the queue — warm-up is on demand.
            if self
                .picker
                .as_ref()
                .is_none_or(|glue| glue.picker.kind != strop_picker::Kind::WorkspaceSymbols)
            {
                self.lsp_state.attach.warm_queue.clear();
                return;
            }
            self.lsp_warm_attach(root, marker);
        }
    }

    /// One unopened project: map its marker to a language, reuse a
    /// live placement, respect refusals, then discover exactly like a
    /// document attach — the same trust and executability gates.
    fn lsp_warm_attach(&mut self, root: PathBuf, marker: String) {
        let Some((language, ext)) = warm_language(&marker) else {
            // The only outcome discovery never reports: the marker
            // pins no language, so the project stays cold (0063 §2).
            self.record_project_status(root, ProjectStatus::ambiguous(&marker));
            return;
        };
        let target = Filesystem::Local;
        let key = attach::AttachKey {
            target: target.clone(),
            language: language.to_string(),
            path: root.clone(),
        };
        if self.lsp_state.attach.attached.iter().any(|attachment| {
            attachment.language == language
                && attachment.root == root
                && attachment.target == target
        }) || self.lsp_state.attach.pending.contains_key(&key)
        {
            // Served or in flight: healthy — clears any stale row.
            self.record_project_status(root, ProjectStatus::healthy());
            return;
        }
        let refusal = self.lsp_state.attach.refused.get(&key).cloned();
        match refusal {
            Some(attach::AttachDecision::TrustRequired { .. })
            | Some(attach::AttachDecision::TrustError { .. })
            | Some(attach::AttachDecision::RemoteIo { .. }) => {}
            Some(decision) => {
                // Sticky refusals are not rediscovered — but the row
                // still reports them (0063 §2).
                self.record_project_status(
                    root,
                    ProjectStatus::from_decision(&decision, &key.language),
                );
                return;
            }
            None => {}
        }
        let ticket = match self.worker_ids.allocate() {
            Ok(ticket) => ticket,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.lsp_state.attach.pending.insert(key.clone(), ticket);
        self.lsp_state.attach.warm_attempts.insert(key);
        let args = attach::AttachArgs {
            ticket,
            path: root.clone(),
            language: language.to_string(),
            target,
        };
        match self.tape.request("lsp.attach", &args) {
            Ok(true) => self.lsp_spawn_discovery(
                ticket,
                strop_workspace::ResourceLocation::local(root),
                ext.to_string(),
                language,
            ),
            Ok(false) => {}
            Err(error) => {
                self.lsp_state.attach.pending.remove(&args_key(&args));
                self.lsp_state.attach.warm_attempts.remove(&args_key(&args));
                self.message = format!("lsp attach diverged from trace: {error}");
            }
        }
    }

    /// A superseded discovery may already have published a live
    /// transport for its server: retire it so the attempt leaves no
    /// orphan process and no undrained event stream.
    /// One warm slot freed by an attach completion (0063 §2).
    pub(super) fn lsp_warm_progress(&mut self) {
        if !self.lsp_state.attach.warm_queue.is_empty() {
            self.lsp_drain_warm_queue();
        }
    }
}

/// Marker file → registry language + probe extension for warm-up
/// (0063 §2). `package.json` stays unmapped: JavaScript vs TypeScript
/// needs configuration evidence a marker alone does not carry.
pub(crate) fn warm_language(marker: &str) -> Option<(&'static str, &'static str)> {
    match marker {
        "Cargo.toml" => Some(("rust", ".rs")),
        "pyproject.toml" | "setup.py" => Some(("python", ".py")),
        "CMakeLists.txt" => Some(("cpp", ".cpp")),
        "go.mod" => Some(("go", ".go")),
        _ => None,
    }
}
