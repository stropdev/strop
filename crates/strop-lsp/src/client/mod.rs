//! The LSP client: the connection handle, spawn/initialize
//! (`spawn.rs`), document sync + requests (`api.rs`), wire helpers
//! (`wire.rs`). async_lsp behind this boundary (0020: tokio stays
//! in the service layer).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use async_lsp::ServerSocket;

use crate::caps::ServerCaps;
use async_lsp::lsp_types::Url;

use crate::protocol::{LspEvent, PendingRequest};

mod api;
mod spawn;
mod trace_io;
mod wire;

/// A live server connection: send from any thread, the runtime thread
/// owns the socket drain. Clone is a second HANDLE on the same
/// connection (socket + channel clone; the mainloop thread is shared —
/// shutdown is idempotent, wait joins once).
///
/// Note: cloning shares `thread` via Arc so a dropped handle never
/// joins out from under the editor's shutdown path.
#[derive(Clone)]
pub struct Client {
    socket: ServerSocket,
    handle: tokio::runtime::Handle,
    tx: Sender<LspEvent>,
    root: PathBuf,
    caps: ServerCaps,
    /// Set once `shutdown()` runs: a server exit afterwards is the clean
    /// protocol exit, not a crash — no Failed event (the demo tape's
    /// `:q!` used to end every LSP session in a fake failure).
    quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The runtime thread running the server mainloop. Joined on
    /// shutdown — dropping the socket while it lives panics inside
    /// async-lsp ("Sender is alive", seen in the demo tape).
    thread: std::sync::Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
    /// Documents opened before initialize completes — flushed on
    /// Initialized (strict servers like pyright drop pre-init opens).
    pending_opens: std::sync::Arc<parking_lot::Mutex<Vec<(PathBuf, String, String)>>>,
    /// goto/hover/switch fired before initialize answered: caps are
    /// unknown (not "no") — queue, flush on Initialized like
    /// pending_opens.
    pending_requests: std::sync::Arc<parking_lot::Mutex<Vec<PendingRequest>>>,
    initialized: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-document didChange versions — the spec requires strictly
    /// increasing, and pyright-family servers enforce it (0014).
    versions: std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<PathBuf, i32>>>,
}
/// clangd's proprietary `textDocument/switchSourceHeader` — not part of
/// the LSP spec, so lsp-types doesn't model it.
enum SwitchSourceHeader {}

impl async_lsp::lsp_types::request::Request for SwitchSourceHeader {
    type Params = async_lsp::lsp_types::TextDocumentIdentifier;
    type Result = Option<async_lsp::lsp_types::Url>;
    const METHOD: &'static str = "textDocument/switchSourceHeader";
}

impl Client {
    /// The LSP exit sequence: shutdown request, then the exit
    /// notification. Called on editor quit — exiting without it makes
    /// servers die with "client exited without proper shutdown" and
    /// paints a fake failure on the statusline.
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

    /// Join the runtime thread after `shutdown()`, with a timeout. A
    /// clean exit drops the client normally; on timeout we leak it on
    /// purpose — a detached thread that outlives the process is cheap,
    /// a dropped-socket panic in the user's terminal is not.
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
