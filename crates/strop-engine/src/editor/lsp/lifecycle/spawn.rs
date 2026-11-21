//! Spawn/discovery orchestration: discovery (config, root, trust,
//! executability) is owned worker work behind the replay gate; the
//! spawned worker's completion lands as an attach record.
use super::super::*;

impl Editor {
    /// Capture the same logical owner live and replayed. Only a live tape
    /// request may use the declaration's native witness.
    pub(super) fn lsp_begin_discovery(
        &mut self,
        args: attach::AttachArgs,
        document: Option<strop_core::id::DocumentId>,
        ext: String,
        language: &'static str,
    ) {
        let member = self.lsp_member_request(&args, document);
        match self.tape.request("lsp.attach", &args) {
            Ok(true) => {
                let mut directory = args.path;
                if document.is_some() {
                    directory.pop();
                }
                self.lsp_spawn_discovery(
                    args.ticket,
                    args.target,
                    directory,
                    ext,
                    language,
                    member,
                );
            }
            Ok(false) => {}
            Err(error) => {
                let key = super::args_key(&args);
                self.lsp_state.attach.pending.remove(&key);
                self.lsp_state.attach.warm_attempts.remove(&key);
                self.lsp_state.attach.members.jobs.remove(&args.ticket);
                self.message = format!("lsp attach diverged from trace: {error}");
            }
        }
    }

    fn lsp_spawn_discovery(
        &mut self,
        ticket: strop_core::worker::WorkerId,
        target: Filesystem,
        directory: PathBuf,
        ext: String,
        language: &'static str,
        member: Option<attach::member::Request>,
    ) {
        let place = match &target {
            Filesystem::Local => attach::DiscoverPlace::Local {
                directory: directory.clone(),
                cwd: self.cwd.clone(),
                git_workdir: self.git.as_ref().map(|g| g.workdir().to_path_buf()),
                member,
            },
            // The remote client is a cheap clone routed to the owned
            // session actor; the document's lease keeps it connected.
            Filesystem::Remote(_) => {
                let Some(file) = self.remote_file().cloned() else {
                    return;
                };
                attach::DiscoverPlace::Remote {
                    file,
                    client: self.remote_client(),
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
            root: directory,
            target,
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
