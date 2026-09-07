//! Owned native side effects. Admission is pure and replay never invokes work.
use super::{Editor, IoEvent};
use std::path::PathBuf;
use strop_core::id::DocumentId;
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    Trust {
        #[serde(with = "strop_core::path_serde")]
        probe: PathBuf,
        #[serde(with = "strop_core::path_serde")]
        cwd: PathBuf,
        #[serde(with = "strop_core::path_serde::option")]
        state_dir: Option<PathBuf>,
    },
    Browser {
        url: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeKey {
    pub document: DocumentId,
    pub focus: u64,
    pub operation: Operation,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum NativeResult {
    Trusted(#[serde(with = "strop_core::path_serde")] PathBuf),
    BrowserRequested,
}

impl Editor {
    pub(crate) fn request_trust(&mut self) {
        if self
            .io
            .native
            .values()
            .any(|key| matches!(key.operation, Operation::Trust { .. }))
        {
            self.message = "trust update already in progress".into();
            return;
        }
        let probe = self
            .buf()
            .path
            .as_ref()
            .map_or_else(|| self.cwd.join("x"), |path| self.cwd.join(path));
        self.request_native(Operation::Trust {
            probe,
            cwd: self.cwd.clone(),
            state_dir: self.state_dir.clone(),
        });
    }
    pub(crate) fn request_browser(&mut self, url: String) {
        self.request_native(Operation::Browser { url });
    }

    fn request_native(&mut self, operation: Operation) {
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: NativeKey {
                document: self.current(),
                focus: self.focus_epoch,
                operation,
            },
        };
        self.io.native.insert(request, ticket.key.clone());
        self.message = match ticket.key.operation {
            Operation::Trust { .. } => "saving trust",
            Operation::Browser { .. } => "opening browser",
        }
        .into();
        match self.tape.request("io.native", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_native(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let operation = ticket.key.operation.clone();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-native",
            move |outcome| {
                let _ = tx.send(IoEvent::Native(Completion { ticket, outcome }));
            },
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(worker::CancelReason::OwnerClosed);
                }
                match operation {
                    Operation::Trust {
                        probe,
                        cwd,
                        state_dir,
                    } => {
                        let root = strop_lsp::registry::workspace_root(&probe, &cwd);
                        match crate::session::trust(state_dir.as_deref(), &root) {
                            Ok(()) => Outcome::Success(NativeResult::Trusted(root)),
                            Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                        }
                    }
                    Operation::Browser { url } => launch_browser(&url),
                }
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(super) fn handle_native(&mut self, completion: Completion<NativeKey, NativeResult>) {
        if self.io.native.get(&completion.ticket.request) != Some(&completion.ticket.key) {
            return;
        }
        self.io.native.remove(&completion.ticket.request);
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        if self.docs.is_empty() || self.current() != key.document || self.focus_epoch != key.focus {
            return;
        }
        match completion.outcome {
            Outcome::Success(NativeResult::Trusted(root)) => {
                self.message = format!("trusted {}", root.display());
                self.lsp_maybe_attach();
            }
            Outcome::Success(NativeResult::BrowserRequested) => {
                self.message = "browser launch requested".into()
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }
}

fn launch_browser(url: &str) -> Outcome<NativeResult> {
    use std::process::{Command, Stdio};
    for opener in ["wslview", "xdg-open", "open"] {
        let mut child = match Command::new(opener)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Outcome::failed(FailureKind::Spawn, format!("{opener}: {error}")),
        };
        // External browser lifetime is deliberately independent of the editor.
        // Reaping is separate from successful launch admission, never input work.
        if let Err(error) = std::thread::Builder::new().name("strop-browser-reaper".into()).spawn(move || {
            if let Err(error) = child.wait() {
                strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({"service":"browser-reaper","error":error.to_string()}));
            }
        }) { return Outcome::failed(FailureKind::ThreadStart, error.to_string()); }
        return Outcome::Success(NativeResult::BrowserRequested);
    }
    Outcome::failed(FailureKind::Unavailable, "no browser opener available")
}
