//! Editor ownership of terminal sessions. PTY handles live on the namespace's
//! admitted worker (0058 WK12); this state holds bounded service handles and
//! immutable publications, and emulation/presentation stay engine-owned.
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
    kind: PrefixKind,
    focus: u64,
    press: strop_core::frontend_input::KeyEvent,
    release: Option<strop_core::frontend_input::KeyEvent>,
}
/// Which editor-owned chord opened the prefix window: the mode escape
/// (`Ctrl-\`, completed by `Ctrl-N`) or the window-command prefix (`Ctrl-W`,
/// vim's terminal `t_CTRL-W` grammar: hjkl/w move panes, N inspects, `.`
/// forwards the literal byte; anything else passes through to the child).
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrefixKind {
    Escape,
    Window,
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

/// The frame half of `terminal_fixture`, factored so tests can publish a
/// later revision through the real update path.
#[cfg(any(test, feature = "test-support"))]
fn fixture_frame(
    session: SessionId,
    rows: &[(&str, strop_terminal::model::Style)],
    revision: u64,
    origin: u64,
) -> Arc<Frame> {
    use strop_terminal::model::{Cell, Cursor, CursorShape, Palette, ProjectedRow, Row, Style};
    use unicode_width::UnicodeWidthChar;
    let display_width = |text: &str| {
        text.chars()
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1))
            .sum::<usize>()
    };
    let columns = rows
        .iter()
        .map(|(text, _)| display_width(text))
        .max()
        .unwrap_or(1);
    let geometry = Geometry {
        columns: columns.max(1) as u16,
        rows: rows.len().max(1) as u16,
        revision,
    };
    let mut projected = Vec::with_capacity(rows.len());
    let mut projection = String::new();
    for (text, style) in rows {
        let mut padded = (*text).to_owned();
        let mut cells: Vec<Cell> = Vec::with_capacity(columns);
        for (index, ch) in text.char_indices() {
            let end = (index + ch.len_utf8()) as u32;
            let width = UnicodeWidthChar::width(ch).unwrap_or(1).clamp(1, 2) as u8;
            cells.push(Cell {
                end,
                width,
                style: *style,
            });
            for _ in 1..width {
                cells.push(Cell {
                    end,
                    width: 0,
                    style: *style,
                });
            }
        }
        while cells.len() < columns {
            padded.push(' ');
            cells.push(Cell {
                end: padded.len() as u32,
                width: 1,
                style: Style::default(),
            });
        }
        let absolute_start = origin + projection.len() as u64;
        projection.push_str(&padded);
        projection.push('\n');
        projected.push(ProjectedRow {
            absolute_start,
            row: Arc::new(Row {
                text: padded,
                cells,
                wrapped: false,
            }),
        });
    }
    let frame = Arc::new(Frame {
        session,
        revision,
        geometry,
        alternate: false,
        cursor: Cursor {
            column: 0,
            row: 0,
            visible: true,
            blinking: false,
            shape: CursorShape::Block,
        },
        palette: Arc::new(Palette::strop()),
        history_rows: 0,
        available_history_rows: 0,
        history_limited: false,
        origin,
        rows: projected.into(),
        projection: ropey::Rope::from_str(&projection),
    });
    frame.validate().unwrap();
    frame
}

#[cfg(any(test, feature = "test-support"))]
impl Editor {
    /// A synthetic terminal with one styled frame and no worker or PTY —
    /// the deterministic shape journeys and surface goldens drive (0065).
    /// Rows are (text, style) pairs mapped like the VT projection: wide
    /// clusters get a continuation cell and every row is space-padded to
    /// the geometry, as `read_row` produces.
    pub fn terminal_fixture(
        &mut self,
        rows: &[(&str, strop_terminal::model::Style)],
        phase: Phase,
    ) -> DocumentId {
        let session = SessionId::from_request(self.worker_ids.allocate().unwrap());
        let frame = fixture_frame(session, rows, 1, 0);
        let geometry = frame.geometry;
        let mut document =
            super::Document::output(strop_core::Buffer::from_snapshot(frame.projection.clone()));
        document.source = super::DocumentSource::Terminal(Box::new(TerminalDocument {
            session,
            frame: Some(frame.clone()),
        }));
        let id = self.docs.try_insert(document).unwrap();
        self.terminals.entries.insert(
            session,
            Entry {
                document: id,
                phase,
                live: Some(frame),
                service: None,
                directory: self.cwd.clone(),
                title: None,
                geometry,
                paste: None,
                keyboard: 0,
            },
        );
        self.switch_to(id);
        id
    }
}
