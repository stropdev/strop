//! Bounded warm-up of eligible unopened projects (0063 §2): the
//! workspace-symbols source reports marker subprojects; each may start
//! its language's server without opening any document. At most four
//! warm attaches are in flight; the rest wait for a completion.
use super::super::super::picker::ProjectStatus;
use super::super::*;
use strop_picker::{WarmHint, WarmProject};

impl Editor {
    /// Bounded warm-up of eligible unopened projects (0063 §2): the
    /// workspace-symbols source reports marker subprojects; each may
    /// start its language's server without opening any document. Warm
    /// servers are reused by the merge tier; servers serving active
    /// documents are never touched (attachments dedup by root).
    pub(crate) fn lsp_warm_scope_projects(&mut self, projects: Vec<WarmProject>) {
        if !self.lsp_state.attach.enabled || self.manifest_loading() {
            return;
        }
        if self
            .picker
            .as_ref()
            .is_none_or(|glue| glue.picker.kind != strop_picker::Kind::WorkspaceSymbols)
        {
            return;
        }
        // The delivery is the bounded list for this observation. Replacing
        // instead of appending retires any queued proposals it superseded.
        self.lsp_state.attach.warm_queue = projects.into();
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
            let Some(project) = self.lsp_state.attach.warm_queue.pop_front() else {
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
            self.lsp_warm_attach(project);
        }
    }

    /// Discovery and declarations only suggest registry IDs. Unknown hints
    /// cannot launch a process or prevent a supported sibling from warming.
    fn lsp_warm_attach(&mut self, WarmProject { root, hint }: WarmProject) {
        let WarmHint::Language(id) = hint else {
            self.record_project_status(root, ProjectStatus::ambiguous("package.json"));
            return;
        };
        let Some((language, ext)) = strop_core::languages::LANGUAGES
            .iter()
            .find(|language| language.name == id)
            .and_then(|language| Some((language.name, *language.extensions.first()?)))
        else {
            self.record_project_status(
                root,
                ProjectStatus::from_decision(&attach::AttachDecision::NoServer, &id),
            );
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
                && (self.lsp_declared_member(&target, &root).is_none()
                    || self
                        .lsp_state
                        .attach
                        .members
                        .servers
                        .contains_key(&attachment.server))
        }) || self.lsp_state.attach.pending.contains_key(&key)
        {
            // Served or in flight: healthy — clears any stale row.
            self.record_project_status(root, ProjectStatus::healthy(language));
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
        self.lsp_state.attach.refused.remove(&key);
        self.lsp_state.attach.pending.insert(key.clone(), ticket);
        self.lsp_state.attach.warm_attempts.insert(key);
        let args = attach::AttachArgs {
            ticket,
            path: root.clone(),
            language: language.to_string(),
            target,
        };
        self.lsp_begin_discovery(args, None, format!(".{ext}"), language);
    }

    /// One warm slot freed by an attach completion (0063 §2).
    pub(crate) fn lsp_warm_progress(&mut self) {
        if !self.lsp_state.attach.warm_queue.is_empty() {
            self.lsp_drain_warm_queue();
        }
    }
}
