//! The LSP connection handle, spawn/initialize, synchronized document
//! lifecycle, ordered wire queue and wire helpers.
use crate::caps::ServerCaps;
use crate::protocol::{LspEvent, ServerId};
use async_lsp::lsp_types::Url;
use async_lsp::ServerSocket;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

mod api;
mod queue;
mod spawn;
mod sync;
#[cfg(test)]
mod tests;
mod trace_io;
mod wire;

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
    root: PathBuf,
    caps: ServerCaps,
    /// A server exit after shutdown is not a crash.
    quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Joined on shutdown: dropping the socket while this lives can panic
    /// inside async-lsp ("Sender is alive").
    thread: std::sync::Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
    /// The ordered wire queue (R6): admission order == wire order.
    queue: queue::WireTx,
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
        self.quitting
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let sock = self.socket.clone();
        self.handle.spawn(async move {
            let _ = sock
                .request::<async_lsp::lsp_types::request::Shutdown>(())
                .await;
            let _ = sock.notify::<async_lsp::lsp_types::notification::Exit>(());
        });
    }

    /// Join after shutdown. On timeout deliberately leak the handle rather
    /// than dropping a live socket and panicking in the terminal.
    pub fn wait(self, timeout: std::time::Duration) {
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        let Some(handle) = handle else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        });
        if rx.recv_timeout(timeout).is_err() {
            std::mem::forget(self);
        }
    }

    fn uri(&self, path: &Path) -> Option<Url> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        Url::from_file_path(abs).ok()
    }
}
