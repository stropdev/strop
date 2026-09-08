//! The ordered wire: one FIFO worker per connection serializes owned
//! rope snapshots and frames them in submission order (R6). Callers
//! cheaply clone a rope snapshot and enqueue; the String
//! materialization LSP full-text sync needs happens here, never on the
//! editor's input thread. `didOpen`/`didChange`/requests/`didClose`
//! therefore reach the server in the order the sync lock admitted them.
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use async_lsp::lsp_types as lt;
use async_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
};
use async_lsp::ServerSocket;
use ropey::Rope;

use crate::caps::ServerCaps;
use crate::protocol::{LspEvent, PendingRequest, ServerId, WireVersion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// One admitted lifecycle step, carrying everything the wire needs.
pub(crate) enum WireJob {
    Open {
        uri: lt::Url,
        language_id: String,
        version: WireVersion,
        text: Rope,
    },
    Change {
        uri: lt::Url,
        version: WireVersion,
        text: Rope,
    },
    Close {
        uri: lt::Url,
    },
    Request(PendingRequest),
}

/// Everything the worker and the request launcher need — deliberately
/// NOT a `Client` clone, so the worker never keeps its own queue sender
/// alive and shutdown-by-drop still terminates it.
#[derive(Clone)]
pub(crate) struct WireEnv {
    pub(crate) id: ServerId,
    pub(crate) name: String,
    pub(crate) hint: String,
    pub(crate) socket: ServerSocket,
    pub(crate) handle: tokio::runtime::Handle,
    pub(crate) tx: Sender<LspEvent>,
    pub(crate) caps: ServerCaps,
    /// Where the server runs: URI mapping and failure labels are
    /// target-aware (local root or remote endpoint+root).
    pub(crate) workspace: crate::target::Workspace,
    pub(crate) sync: Arc<parking_lot::Mutex<super::sync::SyncState>>,
    /// Set by `Client::shutdown`: an exit after this is not a crash.
    pub(crate) quitting: Arc<AtomicBool>,
    /// Set once the server mainloop has ended: stop framing new jobs.
    pub(crate) closed: Arc<AtomicBool>,
}

impl WireEnv {
    /// A failed frame write means the connection is gone; surface it as
    /// a terminal failure unless we asked to quit.
    fn report_dead(&self) {
        if !self.quitting.load(Ordering::Relaxed) {
            let _ = self.tx.send(LspEvent::Failed {
                server: self.id,
                name: self.name.clone(),
                hint: self.hint.clone(),
            });
        }
    }
}

#[derive(Clone)]
pub(crate) struct WireTx {
    tx: Sender<WireJob>,
}

impl WireTx {
    /// Enqueue in admission order. Failing to send means the worker is
    /// gone with the connection; its `Failed` event already terminal.
    pub(crate) fn send(&self, job: WireJob) {
        let _ = self.tx.send(job);
    }
}

/// Start the per-connection wire worker. `None` when the thread cannot
/// be created — callers treat that as a spawn failure.
pub(crate) fn start(env: WireEnv) -> Option<WireTx> {
    let (tx, rx) = channel::<WireJob>();
    let spawned = std::thread::Builder::new()
        .name("strop-lsp-wire".into())
        .spawn(move || worker(env, rx));
    spawned.ok().map(|_| WireTx { tx })
}

fn worker(env: WireEnv, rx: Receiver<WireJob>) {
    while let Ok(job) = rx.recv() {
        // The mainloop is gone: anything still queued can never be
        // framed, and its requests settle through the failure event.
        if env.closed.load(Ordering::Relaxed) {
            break;
        }
        match job {
            WireJob::Open {
                uri,
                language_id,
                version,
                text,
            } => {
                let notified =
                    env.socket
                        .notify::<DidOpenTextDocument>(lt::DidOpenTextDocumentParams {
                            text_document: lt::TextDocumentItem {
                                uri,
                                language_id,
                                version: version.get(),
                                text: text.to_string(),
                            },
                        });
                if notified.is_err() {
                    env.report_dead();
                }
            }
            WireJob::Change { uri, version, text } => {
                let notified =
                    env.socket
                        .notify::<DidChangeTextDocument>(lt::DidChangeTextDocumentParams {
                            text_document: lt::VersionedTextDocumentIdentifier {
                                uri,
                                version: version.get(),
                            },
                            content_changes: vec![lt::TextDocumentContentChangeEvent {
                                range: None,
                                range_length: None,
                                text: text.to_string(),
                            }],
                        });
                if notified.is_err() {
                    env.report_dead();
                }
            }
            WireJob::Close { uri } => {
                // A close for a dying connection needs no failure event.
                let _ = env
                    .socket
                    .notify::<DidCloseTextDocument>(lt::DidCloseTextDocumentParams {
                        text_document: lt::TextDocumentIdentifier { uri },
                    });
            }
            WireJob::Request(request) => {
                // The job order already put every earlier frame on the
                // wire; the request task itself rides the client runtime.
                // Racing runtime teardown cannot abort the process.
                let launch = std::panic::AssertUnwindSafe(|| super::api::launch(&env, request));
                let _ = std::panic::catch_unwind(launch);
            }
        }
    }
}

/// How long the content-modified retry policy waits before re-sending.
pub(crate) const RETRY_DELAY: Duration = Duration::from_millis(800);
