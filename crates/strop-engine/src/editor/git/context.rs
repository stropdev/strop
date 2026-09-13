//! Repository discovery and source-owned context; all native work is a job.
use crate::editor::git_memory::{ContextKey, GitJob};
use crate::editor::Editor;
use crate::files::FileTarget;
use strop_core::worker::{CancelReason, Load, Outcome};
use strop_git::Repo;

impl Editor {
    /// Discover the repository for the current buffer. Native work
    /// runs on a worker (R6); the pure cached context lands through
    /// `GitJob::Context` and invalidates the git view only when it
    /// actually changed. A remote buffer discovers on its endpoint —
    /// `rev-parse --show-toplevel` from the file's directory, then the
    /// context reads — through the bounded remote-execution boundary;
    /// the local cwd is never consulted for it (0036 RW8).
    pub fn discover_git(&mut self) {
        if self.docs.is_empty() || self.finishing {
            return;
        }
        if let Some(context) = self.cur().git_context().cloned() {
            if let Load::Running(ticket) = &self.git_discovery {
                self.cancel_git_worker(ticket.request, CancelReason::Superseded);
            }
            self.git_discovery = Load::Idle;
            if self.git.as_ref() != Some(&context) {
                self.git = Some(context);
                self.invalidate_git_view();
            }
            return;
        }
        self.git_discovery.retry_failed();
        let directory = self.directory().map(|source| source.location.clone());
        if let Some(directory) = &directory {
            match &directory.filesystem {
                strop_workspace::Filesystem::Remote(endpoint) => {
                    match strop_workspace::RemoteFile::from_path(
                        endpoint.clone(),
                        directory.path.clone(),
                    ) {
                        Ok(file) => self.discover_remote_git(file),
                        Err(error) => self.message = error.to_string(),
                    }
                    return;
                }
                strop_workspace::Filesystem::Container(container) => {
                    self.discover_container_git(container.clone(), directory.path.clone());
                    return;
                }
                strop_workspace::Filesystem::Local => {}
            }
        }
        if let Some(file) = self.remote_file().cloned() {
            self.discover_remote_git(file);
            return;
        }
        // A container document discovers its repository inside the
        // container (0037 DC1b) — the local cwd is never consulted.
        if let crate::editor::document::DocumentSource::Container { container, path } =
            &self.cur().source
        {
            self.discover_container_git(container.clone(), path.clone());
            return;
        }
        let from = directory
            .map(|directory| directory.path)
            .or_else(|| self.buf().path.clone())
            .unwrap_or_else(|| self.cwd.clone());
        // one request per origin while running; a resolved (Ready)
        // discovery is re-derivable, so explicit switches refresh it
        if matches!(&self.git_discovery, Load::Running(current) if current.key.from == FileTarget::Local(from.clone()))
        {
            return;
        }
        let running = match &self.git_discovery {
            Load::Running(current) => Some(current.request),
            _ => None,
        };
        if let Some(request) = running {
            self.cancel_git_worker(request, CancelReason::Superseded);
        }
        let Some(ticket) = self.git_ticket(ContextKey {
            from: FileTarget::Local(from.clone()),
        }) else {
            return;
        };
        self.git_discovery = Load::Running(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"discover","target":"local","from":from.to_string_lossy(),
            })
        });
        let args = ticket.clone();
        self.launch_git_job(
            "git-discover",
            "git.discover",
            ticket,
            &args,
            GitJob::Context,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                Outcome::Success(Repo::discover(&from).map(|repo| repo.context()))
            },
        );
    }

    /// Remote discovery: the repository containing the current remote
    /// file, on its endpoint. `Ok(None)` (no repository there) is an
    /// honest answer, published like any other context.
    fn discover_remote_git(&mut self, file: strop_workspace::RemoteFile) {
        let endpoint = file.endpoint().clone();
        let from_dir = if self.directory().is_some() {
            file.path().to_owned()
        } else {
            file.path()
                .parent()
                .unwrap_or(std::path::Path::new("/"))
                .to_owned()
        };
        let from = FileTarget::Remote(file.into());
        if matches!(&self.git_discovery, Load::Running(current) if current.key.from == from) {
            return;
        }
        let running = match &self.git_discovery {
            Load::Running(current) => Some(current.request),
            _ => None,
        };
        if let Some(request) = running {
            self.cancel_git_worker(request, CancelReason::Superseded);
        }
        let Some(ticket) = self.git_ticket(ContextKey { from }) else {
            return;
        };
        self.git_discovery = Load::Running(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"discover","target":"remote",
                "endpoint":endpoint.to_string(),"from":from_dir.to_string_lossy(),
            })
        });
        let args = ticket.clone();
        self.launch_git_job(
            "git-discover-remote",
            "git.discover",
            ticket,
            &args,
            GitJob::Context,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                match strop_git::remote::discover(&endpoint, &from_dir, &cancel) {
                    Ok(None) => Outcome::Success(None),
                    Ok(Some(workdir)) => {
                        match strop_git::remote::context(&endpoint, &workdir, &cancel) {
                            Ok(context) => Outcome::Success(Some(context)),
                            Err(error) => Outcome::Failed {
                                failure: strop_core::worker::Failure::new(
                                    strop_core::worker::FailureKind::Exit,
                                    error.to_string(),
                                ),
                                partial: None,
                            },
                        }
                    }
                    Err(error) => Outcome::Failed {
                        failure: strop_core::worker::Failure::new(
                            strop_core::worker::FailureKind::Exit,
                            error.to_string(),
                        ),
                        partial: None,
                    },
                }
            },
        );
    }
    /// Container discovery (0037 DC1b): the repository containing the
    /// current container document, inside that container. `Ok(None)` (no
    /// repository there) is an honest answer, published like any other.
    fn discover_container_git(
        &mut self,
        container: strop_workspace::ContainerId,
        path: std::path::PathBuf,
    ) {
        let from_dir = if self.directory().is_some() {
            path.clone()
        } else {
            path.parent()
                .unwrap_or(std::path::Path::new("/"))
                .to_owned()
        };
        let from = FileTarget::Container {
            container: container.clone(),
            path: from_dir.clone(),
        };
        if matches!(&self.git_discovery, Load::Running(current) if current.key.from == from) {
            return;
        }
        let running = match &self.git_discovery {
            Load::Running(current) => Some(current.request),
            _ => None,
        };
        if let Some(request) = running {
            self.cancel_git_worker(request, CancelReason::Superseded);
        }
        let Some(ticket) = self.git_ticket(ContextKey { from }) else {
            return;
        };
        self.git_discovery = Load::Running(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"discover","target":"container",
                "container":container.to_string(),"from":from_dir.to_string_lossy(),
            })
        });
        let args = ticket.clone();
        self.launch_git_job(
            "git-discover-container",
            "git.discover",
            ticket,
            &args,
            GitJob::Context,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let fail = |error: &dyn ToString| Outcome::Failed {
                    failure: strop_core::worker::Failure::new(
                        strop_core::worker::FailureKind::Exit,
                        error.to_string(),
                    ),
                    partial: None,
                };
                match strop_git::container::discover(&container, &from_dir, &cancel) {
                    Ok(None) => Outcome::Success(None),
                    Ok(Some(workdir)) => {
                        match strop_git::container::context(&container, &workdir, &cancel) {
                            Ok(context) => Outcome::Success(Some(context)),
                            Err(error) => fail(&error),
                        }
                    }
                    Err(error) => fail(&error),
                }
            },
        );
    }

    pub(crate) fn git_context(&self) -> Option<&strop_git::GitContext> {
        self.panes
            .get(self.active_pane)
            .and_then(|pane| self.docs.get(pane.doc))
            .and_then(crate::editor::Document::git_context)
            .or(self.git.as_ref())
    }
}
