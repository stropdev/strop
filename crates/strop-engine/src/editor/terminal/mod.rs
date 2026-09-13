//! Editor ownership of local terminals. Native handles remain on strop-terminal's
//! worker; this state holds bounded service handles and immutable publications.
mod input;
mod lifecycle;
mod presentation;
mod privacy;
#[cfg(test)]
mod tests;
use super::{Editor, Pane};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc},
};
use strop_core::id::DocumentId;
use strop_terminal::{
    model::{Frame, Geometry, Phase, SessionId},
    service::Service,
};

#[derive(Debug, Clone)]
pub struct TerminalDocument {
    pub session: SessionId,
    /// The buffer and all Normal inspection views use this same generation.
    pub frame: Option<Arc<Frame>>,
}
struct Entry {
    document: DocumentId,
    phase: Phase,
    live: Option<Arc<Frame>>,
    service: Option<Service>,
    directory: PathBuf,
    title: Option<String>,
    geometry: Geometry,
    paste: Option<u64>,
    keyboard: u8,
}
struct Prefix {
    session: SessionId,
    focus: u64,
    press: strop_core::frontend_input::KeyEvent,
    release: Option<strop_core::frontend_input::KeyEvent>,
}
pub(crate) struct State {
    entries: HashMap<SessionId, Entry>,
    tx: mpsc::Sender<SessionId>,
    pub(super) rx: Option<mpsc::Receiver<SessionId>>,
    prefix: Option<Prefix>,
    focused: bool,
    pub capture: bool,
    pub keyboard: u8,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            tx,
            rx: Some(rx),
            prefix: None,
            capture: false,
            focused: true,
            keyboard: 0,
        }
    }
}
impl State {
    pub fn pending(&self) -> bool {
        self.entries.values().any(|entry| {
            matches!(entry.phase, Phase::Starting | Phase::Closing)
                || entry.service.as_ref().is_some_and(Service::pending)
        })
    }
    pub fn live(&self) -> bool {
        self.entries.values().any(|entry| entry.phase.live())
    }
}
impl Editor {
    pub fn terminal_document(&self, document: DocumentId) -> Option<&TerminalDocument> {
        match &self.doc(document).source {
            super::DocumentSource::Terminal(terminal) => Some(terminal),
            _ => None,
        }
    }
    pub fn terminal_input_active(&self) -> bool {
        self.panes
            .get(self.active_pane)
            .is_some_and(|pane| self.terminal_view_input(pane))
    }
    pub fn terminal_view_input(&self, pane: &Pane) -> bool {
        pane.terminal_input
            && self.terminal_document(pane.doc).is_some_and(|document| {
                self.terminals
                    .entries
                    .get(&document.session)
                    .is_some_and(|entry| entry.phase.live())
            })
    }
    pub fn terminal_frame(&self, document: DocumentId, live: bool) -> Option<&Arc<Frame>> {
        let terminal = self.terminal_document(document)?;
        if live {
            self.terminals.entries.get(&terminal.session)?.live.as_ref()
        } else {
            terminal.frame.as_ref()
        }
    }
    pub fn terminal_phase(&self, document: DocumentId) -> Option<&Phase> {
        let terminal = self.terminal_document(document)?;
        Some(&self.terminals.entries.get(&terminal.session)?.phase)
    }
    pub fn terminal_launch_directory(&self, document: DocumentId) -> Option<&std::path::Path> {
        let terminal = self.terminal_document(document)?;
        Some(&self.terminals.entries.get(&terminal.session)?.directory)
    }
}
