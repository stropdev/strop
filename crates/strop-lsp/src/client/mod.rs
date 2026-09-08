//! The LSP connection handle, spawn/initialize, synchronized document
//! lifecycle, ordered wire queue and wire helpers.
use crate::caps::ServerCaps;
use crate::protocol::{LspEvent, ServerId};
use crate::target::Workspace;
use async_lsp::lsp_types::Url;
use async_lsp::ServerSocket;
use std::path::Path;
use std::sync::mpsc::Sender;

mod api;
mod process;
mod queue;
mod spawn;
mod sync;
#[cfg(test)]
mod tests;

mod trace_io;
mod wire;

pub use spawn::SpawnError;

/// Clones share one connection, its wire queue and runtime thread.
/// Shutdown is idempotent; wait joins once, never underneath another
/// handle's shutdown path.
#[derive(Clone)]
pub struct Client {
    id: ServerId,
    next_request: std::sync::Arc<std::sync::atomic::AtomicU64>,
    sync: std::sync::Arc<parking_lot::Mutex<sync::SyncState>>,
    socket: ServerSocket,
    handle: tokio::runtime::Handle,
    tx: Sender<LspEvent>,
    /// Where this server runs and which filesystem its paths name:
    /// local disk or one remote endpoint (0036 RW8).
    workspace: Workspace,
    caps: ServerCaps,
    /// A server exit after shutdown is not a crash.
    quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Joined on shutdown: dropping the socket while this lives can panic
    /// inside async-lsp ("Sender is alive").
    thread: std::sync::Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
    /// The ordered wire queue (R6): admission order == wire order.
    queue: queue::WireTx,
    stop: std::sync::Arc<ServiceStop>,
}

struct ServiceStop(parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>);
impl ServiceStop {
    fn halt(&self) {
        if let Some(signal) = self.0.lock().take() {
            let _ = signal.send(());
        }
    }
}
impl Drop for ServiceStop {
    fn drop(&mut self) {
        self.halt();
    }
}

/// clangd's proprietary extension, absent from lsp-types.
enum SwitchSourceHeader {}
impl async_lsp::lsp_types::request::Request for SwitchSourceHeader {
    type Params = async_lsp::lsp_types::TextDocumentIdentifier;
    type Result = Option<async_lsp::lsp_types::Url>;
    const METHOD: &'static str = "textDocument/switchSourceHeader";
}

impl Client {
    pub fn id(&self) -> ServerId {
        self.id
    }

    /// The LSP exit sequence: shutdown request, then exit notification.
    pub fn shutdown(&self) {
        if self
            .quitting
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let sock = self.socket.clone();
        let stop = self.stop.clone();
        self.handle.spawn(async move {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                sock.request::<async_lsp::lsp_types::request::Shutdown>(()),
            )
            .await;
            let _ = sock.notify::<async_lsp::lsp_types::notification::Exit>(());
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            stop.halt();
        });
    }

    /// Join once after shutdown. A timeout revokes the native mainloop; the
    /// process owner completes asynchronous group cleanup without leaking clients.
    pub fn wait(self, timeout: std::time::Duration) {
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        let Some(handle) = handle else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        });
        if rx.recv_timeout(timeout).is_err() {
            self.stop.halt();
        }
    }

    fn uri(&self, path: &Path) -> Option<Url> {
        self.workspace.uri(path)
    }

    /// Where this server runs: local root or remote endpoint+root.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }
}
