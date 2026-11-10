//! Editor ownership of terminal sessions (0090 §4, Neovim's model). A
//! terminal is a live buffer: every published frame lands in the
//! document, NORMAL views follow output while their cursor sits on the
//! last line, and TERMINAL mode hands every key to the child. The PTY
//! runs in-process for local sessions and on the namespace's admitted
//! worker for remote ones; this state holds bounded service handles and
//! the installed frame, and emulation/presentation stay engine-owned.
pub(crate) mod actions;
mod input;
mod lifecycle;
mod presentation;
mod privacy;
#[cfg(test)]
mod pty_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod theme_tests;
use super::{Editor, Pane};
use std::{
    collections::HashMap,
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
    /// Immutable admitted launch identity. Shell titles and reported cwd never mutate it.
    pub launch: strop_workspace::ResourceLocation,
    /// The published frame whose projection is the buffer's text — the
    /// one generation every view of the document reads.
    pub frame: Option<Arc<Frame>>,
}

/// The view a terminal buffer left behind when the user switched away
/// (0090 §4: re-entry restores cursor and viewport, never 1:1).
/// Positions are stream bytes — `origin + byte` — so a frame whose
/// scrollback moved on still maps them to the same content.
#[derive(Debug, Clone, Copy)]
struct SavedView {
    head: u64,
    top: u64,
    /// The cursor was on the last line (or the view was in TERMINAL
    /// mode): re-entry keeps following output.
    following: bool,
}

struct Entry {
    document: DocumentId,
    phase: Phase,
    service: Option<Service>,
    launch: strop_workspace::ResourceLocation,
    title: Option<String>,
    geometry: Geometry,
    keyboard: u8,
    view: Option<SavedView>,
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
    /// The last issued session number. Sessions number from 1 per editor
    /// (0088 U12): the number is what `term://…//N:` shows, and it never
    /// repeats, so a stale record can never bind to a later launch.
    last_session: u64,
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
            last_session: 0,
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
    /// Live sessions, for the `:qa` notice.
    pub fn live_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.phase.live())
            .count()
    }
    /// The next per-editor session number; `None` once the space is spent.
    fn next_session(&mut self) -> Option<SessionId> {
        self.last_session = self.last_session.checked_add(1)?;
        Some(SessionId::new(self.last_session))
    }
}

/// Neovim's buffer name for a terminal: `term://{cwd}//{N}:{program}`,
/// where `N` is the per-editor session number (there is no pid for a
/// remote child) and `program` is the launched command or shell.
pub(crate) fn terminal_name(
    session: SessionId,
    launch: &strop_workspace::ResourceLocation,
    program: &str,
) -> String {
    let directory = match launch.filesystem {
        strop_workspace::Filesystem::Local => {
            strop_workspace::directory::display_path(&launch.path)
        }
        _ => launch.label(),
    };
    format!(
        "term://{directory}//{}:{}",
        session.get(),
        strop_core::layout::printable_text(program)
    )
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
    /// The installed frame: the latest publication for a live session,
    /// the final one for an exited session.
    pub fn terminal_frame(&self, document: DocumentId) -> Option<&Arc<Frame>> {
        self.terminal_document(document)?.frame.as_ref()
    }
    pub fn terminal_phase(&self, document: DocumentId) -> Option<&Phase> {
        let terminal = self.terminal_document(document)?;
        Some(&self.terminals.entries.get(&terminal.session)?.phase)
    }
    pub fn terminal_launch_location(
        &self,
        document: DocumentId,
    ) -> Option<&strop_workspace::ResourceLocation> {
        let terminal = self.terminal_document(document)?;
        Some(&self.terminals.entries.get(&terminal.session)?.launch)
    }
    pub fn terminal_launch_directory(&self, document: DocumentId) -> Option<&std::path::Path> {
        self.terminal_launch_location(document)
            .map(|location| location.path.as_path())
    }
}

/// The frame half of `terminal_fixture`, factored so tests can publish a
/// later revision through the real update path.
#[cfg(any(test, feature = "test-support"))]
fn fixture_frame(
    session: SessionId,
    theme: strop_core::theme::ThemeId,
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
        palette: Arc::new(Palette::for_theme(theme)),
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
    /// A synthetic terminal with one styled frame and no PTY — the
    /// deterministic shape journeys and surface goldens drive (0065).
    /// Rows are (text, style) pairs mapped like the VT projection: wide
    /// clusters get a continuation cell and every row is space-padded to
    /// the geometry, as `read_row` produces.
    pub fn terminal_fixture(
        &mut self,
        rows: &[(&str, strop_terminal::model::Style)],
        phase: Phase,
    ) -> DocumentId {
        self.terminal_fixture_at(
            strop_workspace::ResourceLocation::local(self.cwd.clone()),
            rows,
            phase,
        )
    }

    /// A synthetic terminal with an explicit immutable launch identity.
    pub fn terminal_fixture_at(
        &mut self,
        launch: strop_workspace::ResourceLocation,
        rows: &[(&str, strop_terminal::model::Style)],
        phase: Phase,
    ) -> DocumentId {
        let session = self.terminals.next_session().unwrap();
        let frame = fixture_frame(session, self.presentation.theme, rows, 1, 0);
        let geometry = frame.geometry;
        let mut buffer = strop_core::Buffer::from_snapshot(frame.projection.clone());
        buffer.name = Some(terminal_name(session, &launch, "sh"));
        let mut document = super::Document::output(buffer);
        document.source = super::DocumentSource::Terminal(Box::new(TerminalDocument {
            session,
            launch: launch.clone(),
            frame: Some(frame),
        }));
        let id = self.docs.try_insert(document).unwrap();
        self.terminals.entries.insert(
            session,
            Entry {
                document: id,
                phase,
                service: None,
                launch,
                title: None,
                geometry,
                keyboard: 0,
                view: None,
            },
        );
        self.switch_to(id);
        id
    }

    pub fn terminal_fixture_publish(
        &mut self,
        document: DocumentId,
        rows: &[(&str, strop_terminal::model::Style)],
    ) {
        let Some(source) = self.terminal_document(document) else {
            return;
        };
        let session = source.session;
        let Some((revision, origin)) = source
            .frame
            .as_ref()
            .map(|frame| (frame.revision.saturating_add(1), frame.origin))
        else {
            return;
        };
        let Some(phase) = self
            .terminals
            .entries
            .get(&session)
            .map(|entry| entry.phase.clone())
        else {
            return;
        };
        self.apply_terminal_update(strop_terminal::model::Update {
            session,
            phase,
            frame: Some(fixture_frame(
                session,
                self.presentation.theme,
                rows,
                revision,
                origin,
            )),
            effects: Vec::new(),
            acknowledged_input: 0,
            warning: None,
        });
    }

    /// End a fixture session with an exit code, through the real update path.
    pub fn terminal_fixture_exit(&mut self, document: DocumentId, code: i32) {
        let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        else {
            return;
        };
        self.apply_terminal_update(strop_terminal::model::Update {
            session,
            phase: Phase::Exited {
                code: Some(code),
                signal: None,
            },
            frame: None,
            effects: Vec::new(),
            acknowledged_input: 0,
            warning: None,
        });
    }
}
