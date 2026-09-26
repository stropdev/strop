//! Spawn/discovery orchestration: discovery (config, root, trust,
//! executability) is owned worker work behind the replay gate; the
//! spawned worker's completion lands as an attach record.
use super::super::*;

impl Editor {
    pub(super) fn lsp_spawn_discovery(
        &mut self,
        ticket: strop_core::worker::WorkerId,
        doc: ResourceLocation,
        ext: String,
        language: &'static str,
    ) {
        let place = match doc.filesystem.clone() {
            Filesystem::Local => attach::DiscoverPlace::Local {
                abs: doc.path.clone(),
                cwd: self.cwd.clone(),
                git_workdir: self.git.as_ref().map(|g| g.workdir().to_path_buf()),
                worker: self.filesystem.worker().clone(),
            },
            // The remote client is a cheap clone routed to the owned
            // session actor; the document's lease keeps it connected.
            // The endpoint's admitted worker lease rides along when the
            // session holds one (0058 WK10): read-only, never deploying.
            Filesystem::Remote(_) => {
                let Some(file) = self.remote_file().cloned() else {
                    return;
                };
                let worker = self.remote.workers.get(file.endpoint());
                attach::DiscoverPlace::Remote {
                    file,
                    client: self.remote_client(),
                    worker,
                }
            }
            // The container workspace roots at the document's directory;
            // the id is the canonical inspect identity.
            Filesystem::Container(id) => {
                let worker = self
                    .containers
                    .attached
                    .get(id.as_str())
                    .and_then(|identity| self.containers.workers.get(identity))
                    .map(|lease| lease.worker().clone());
                attach::DiscoverPlace::Container {
                    root: doc.path.parent().unwrap_or(Path::new("/")).to_path_buf(),
                    id,
                    worker,
                }
            }
        };
        let input = attach::DiscoverInput {
            ticket,
            place,
            ext,
            language,
            state_dir: self.state_dir.clone(),
            xdg: strop_lsp::languages::xdg_path(),
            transport: self.lsp_state.attach.transport.clone(),
        };
        let cancelled = attach::AttachRecord {
            ticket,
            server: None,
            language: language.to_owned(),
            name: language.to_owned(),
            root: doc.path.parent().unwrap_or(Path::new("/")).to_owned(),
            target: doc.filesystem,
            outcome: attach::AttachDecision::Cancelled,
            layers: Vec::new(),
        };
        let done = self.lsp_state.attach.attach_channel();
        // Discovery is an owned worker: the remote reads and probes it
        // performs are cancellable (superseded attach attempts are
        // cancelled when a newer ticket takes the key).
        let handle = strop_core::worker::spawn(
            "strop-lsp-attach",
            move |outcome| {
                let record = match outcome {
                    strop_core::worker::Outcome::Success(record) => record,
                    strop_core::worker::Outcome::Cancelled(_) => cancelled,
                    strop_core::worker::Outcome::Failed { failure, .. } => attach::AttachRecord {
                        outcome: attach::AttachDecision::SpawnFailed {
                            reason: failure.message,
                        },
                        ..cancelled
                    },
                };
                let _ = done.send(record);
            },
            move |token| match attach::discover(input, &token) {
                Some(record) => strop_core::worker::Outcome::Success(record),
                None => strop_core::worker::Outcome::Cancelled(
                    strop_core::worker::CancelReason::OwnerClosed,
                ),
            },
        );
        self.worker_handles.insert(ticket, handle);
    }
}
