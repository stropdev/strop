//! Model ownership of declared placements. Native directory evidence lives
//! on the discovery worker; replay uses only the recorded terminal decision.
use super::super::attach::member::{Job, Request};
use super::super::*;
use std::sync::{atomic::Ordering, Arc, OnceLock};
use strop_core::id::DocumentId;

impl Editor {
    /// Watch hints are advisory. Creating/removing/renaming a directory can
    /// move an entire child without a separate event for its nested markers.
    /// Check only the bounded declaration list; never stat an event path.
    pub(crate) fn manifest_binding_hint(&self, relative: &Path, namespace_changed: bool) -> bool {
        let declarations = self.workspace_declarations();
        if declarations.is_empty() {
            return false;
        }
        let name = relative.file_name().unwrap_or_default();
        let policy = strop_picker::ProjectCatalog::is_marker(name)
            || matches!(
                name.to_str(),
                Some(".gitignore" | ".ignore" | ".rgignore" | ".strop")
            )
            || relative.ends_with(".git/info/exclude")
            || relative.ends_with(".strop/languages.toml");
        if !policy && !namespace_changed {
            return false;
        }
        // A .git/info/exclude or project layer can affect all descendants
        // of its owning repository/project, not just its control directory.
        let owner = if relative.ends_with(".git/info/exclude") {
            relative.ancestors().nth(3)
        } else if relative.ends_with(".strop/languages.toml") {
            relative.ancestors().nth(2)
        } else {
            relative.parent()
        }
        .unwrap_or(Path::new(""));
        declarations
            .iter()
            .any(|d| relative.starts_with(d.path()) || (policy && d.path().starts_with(owner)))
    }

    pub(super) fn lsp_declared_member(&self, target: &Filesystem, path: &Path) -> Option<usize> {
        if *target != Filesystem::Local {
            return None;
        }
        let relative = path.strip_prefix(&self.cwd).ok()?;
        self.workspace_declarations()
            .iter()
            .enumerate()
            .filter(|(_, d)| relative.starts_with(d.path()))
            .max_by_key(|(_, d)| d.path().as_os_str().len())
            .map(|(index, _)| index)
    }

    pub(super) fn lsp_member_request(
        &mut self,
        args: &attach::AttachArgs,
        document: Option<DocumentId>,
    ) -> Option<Request> {
        if args.target != Filesystem::Local || !args.path.starts_with(&self.cwd) {
            return None;
        }
        let index = self.lsp_declared_member(&args.target, &args.path);
        let declarations = self.workspace_declarations().clone();
        let attach = &mut self.lsp_state.attach;
        let candidates = attach
            .attached
            .iter()
            .filter(|a| {
                a.target == args.target
                    && a.language == args.language
                    && args.path.starts_with(&a.root)
            })
            .filter_map(|a| Some((a.server, attach.members.servers.get(&a.server)?.clone())))
            .collect();
        let witness = Arc::new(OnceLock::new());
        attach.members.jobs.insert(
            args.ticket,
            Job {
                declared: index.is_some(),
                document,
                witness: witness.clone(),
            },
        );
        Some(Request {
            declarations,
            index,
            target: args.path.clone(),
            warm: document.is_none(),
            retired: if index.is_some() {
                attach.members.retired.clone()
            } else {
                Arc::default()
            },
            witness,
            candidates,
        })
    }

    /// New input invalidates every observation made using the old declaration.
    /// On installing declarations, also revoke earlier *ordinary* pending
    /// attempts/bindings for their paths, not another local or remote project.
    pub(crate) fn retire_manifest_attachments(&mut self) {
        let roots: Vec<_> = self
            .workspace_declarations()
            .iter()
            .map(|d| self.cwd.join(d.path()))
            .collect();
        let attach = &mut self.lsp_state.attach;
        std::mem::take(&mut attach.members.retired).store(true, Ordering::Release);
        let old_roots = std::mem::replace(&mut attach.members.roots, roots);
        let roots = &attach.members.roots;
        let covered = |path: &Path| roots.iter().any(|r| path.starts_with(r));
        let affected =
            |key: &attach::AttachKey| key.target == Filesystem::Local && covered(&key.path);
        attach
            .warm_queue
            .retain(|p| !covered(&p.root) && !old_roots.iter().any(|r| p.root.starts_with(r)));
        attach.pending.retain(|key, id| {
            !affected(key) && !attach.members.jobs.get(id).is_some_and(|j| j.declared)
        });
        attach.members.jobs.retain(|_, job| !job.declared);
        attach
            .warm_attempts
            .retain(|key| attach.pending.contains_key(key));
        let refused = std::mem::take(&mut attach.members.refused);
        attach
            .refused
            .retain(|key, _| !affected(key) && !refused.contains(key));
        attach
            .trust_roots
            .retain(|key, _| !affected(key) && !refused.contains(key));
        let mut servers: Vec<_> = attach
            .attached
            .iter()
            .filter(|a| {
                attach.members.declaration_servers.contains(&a.server)
                    || (a.target == Filesystem::Local && roots.contains(&a.root))
            })
            .map(|a| a.server)
            .collect();
        servers.sort_by_key(|server| server.get());
        attach.members.documents.retain(|_, (path, server)| {
            !covered(path) && !attach.members.declaration_servers.contains(server)
        });
        let mut documents: Vec<_> = self
            .lsp_state
            .bindings
            .iter()
            .filter(|(_, b)| {
                b.target == Filesystem::Local
                    && (covered(&b.path) || attach.members.declaration_servers.contains(&b.server))
            })
            .map(|(id, _)| *id)
            .collect();
        documents.sort();
        for document in documents {
            self.lsp_close_document(document);
        }
        for server in servers {
            self.lsp_retire_server(server, "workspace declaration changed");
        }
    }

    pub(super) fn lsp_accept_member_file(
        &mut self,
        job: &Job,
        key: &attach::AttachKey,
        server: ServerId,
    ) {
        let Some(document) = job.document else { return };
        if self
            .lsp_document_location(document)
            .is_some_and(|doc| doc.filesystem == key.target && doc.path == key.path)
        {
            if self
                .lsp_state
                .bindings
                .get(&document)
                .is_some_and(|b| b.server != server || b.path != key.path)
            {
                self.lsp_close_document(document);
            }
            self.lsp_state.jump_contexts.remove(&document);
            self.lsp_state
                .attach
                .members
                .documents
                .insert(document, (key.path.clone(), server));
            self.lsp_did_open_document(document);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::worker::WorkerId;

    #[test]
    fn retirement_rejects_old_ticket_and_keeps_independent_local_and_remote_placements() {
        let mut editor = Editor::new_in(strop_core::Buffer::from_text(""), "/work".into());
        let server = ServerId::new(10);
        let ticket = WorkerId::new(11);
        let member = PathBuf::from("/work/child");
        let key = attach::AttachKey {
            target: Filesystem::Local,
            language: "rust".into(),
            path: member.clone(),
        };
        let attach = &mut editor.lsp_state.attach;
        attach.pending.insert(key.clone(), ticket);
        attach.warm_attempts.insert(key);
        attach.members.jobs.insert(
            ticket,
            Job {
                declared: true,
                document: None,
                witness: Arc::default(),
            },
        );
        attach.members.servers.insert(server, Arc::default());
        attach.members.declaration_servers.insert(server);
        attach.members.roots.push(member.clone());
        let old = attach.members.retired.clone();
        let remote = Filesystem::Remote(
            strop_workspace::RemoteEndpoint::parse("ssh://example.com").unwrap(),
        );
        for (id, target, root) in [
            (10, Filesystem::Local, member.clone()),
            (12, Filesystem::Local, PathBuf::from("/work/other")),
            (13, remote.clone(), member.clone()),
        ] {
            attach.attached.push(attach::Attachment {
                server: ServerId::new(id),
                target,
                root,
                language: "rust".into(),
            });
        }
        editor.retire_manifest_attachments();
        assert!(old.load(Ordering::Acquire));
        editor.handle_lsp_attach(attach::AttachRecord {
            ticket,
            server: Some(ServerId::new(14)),
            name: "held".into(),
            language: "rust".into(),
            root: member,
            target: Filesystem::Local,
            outcome: attach::AttachDecision::Attached,
            layers: vec![],
        });
        let remaining = &editor.lsp_state.attach.attached;
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].server, ServerId::new(12));
        assert_eq!(remaining[1].target, remote);
        assert!(editor.lsp_state.attach.pending.is_empty());
        assert!(editor.lsp_state.attach.warm_attempts.is_empty());
    }
}
