//! The deterministic driver (R11): live drivers record one `Action` before
//! the reducer or render work runs; replay consumes those actions and
//! re-runs the SAME production handlers with native launches suppressed by
//! the tape. Service interleaving is the consumption order at the single
//! delivery boundary — never worker-completion wall time.
use std::io;

use serde::{Deserialize, Serialize};

use crate::editor::events::AppEvent;
use crate::editor::Editor;
use strop_trace::replay::Tick;

#[derive(Serialize, Deserialize)]
pub struct StartupOpen {
    pub target: crate::files::FileTarget,
    pub line: Option<strop_core::id::LineIndex>,
    pub view: super::super::remote::RemoteView,
}

/// Every external step a replay reproduces. `Event` carries the shared
/// typed `AppEvent` (terminal input, paste, resize, quit intent, and every
/// completed worker/service result), so the live handler and the replayed
/// handler are one function.
#[derive(Serialize, Deserialize)]
pub enum Action {
    /// The explicit start-services step after the seed: git discovery
    /// registration, the optional directory picker, LSP start state.
    Start {
        directory_picker: bool,
        #[serde(default)]
        open: Option<StartupOpen>,
    },
    /// One external delivery, recorded at handler entry — before stale
    /// filters or acceptance decisions, so rejected results replay too.
    Event(AppEvent),
    /// One rendered frame: the dimensions are inputs, the canonical cell
    /// observation is checked inside the shared draw.
    Frame { columns: u16, rows: u16 },
    /// Deliberate shutdown: session persistence is requested here and its
    /// terminal publication must land as an Event before the tape's `End`.
    Finish,
}

impl Editor {
    /// Live drivers: record the action, then run the shared body.
    pub(crate) fn recorded_action(&mut self, action: Action, tick: Tick) -> io::Result<()> {
        self.tape.action(tick, &action)?;
        self.apply_recorded(action)
    }

    /// The one body both modes execute. Native side effects are confined
    /// to the tape-gated call sites inside these handlers.
    pub(crate) fn apply_recorded(&mut self, action: Action) -> io::Result<()> {
        let frame = matches!(action, Action::Frame { .. });
        match action {
            Action::Start {
                directory_picker,
                open,
            } => {
                let startup_message = self.message.clone();
                if let Some(open) = open {
                    self.lsp_start_services();
                    let intent = super::super::io::OpenIntent::RemoteView {
                        view: open.view,
                        line: open.line,
                    };
                    self.request_target(open.target, intent);
                } else {
                    self.discover_git();
                    if directory_picker {
                        self.open_picker(strop_picker::Kind::Files);
                    }
                    self.lsp_start_services();
                }
                if !startup_message.is_empty() {
                    self.message = startup_message;
                }
            }
            Action::Event(event) => {
                self.handle_app_event(event);
                if !self.docs.is_empty() {
                    self.lsp_sync_changed();
                }
                // Logical OSC52 consumption is common to both modes; the
                // actual escape write stays the terminal adapter's
                // live-only effect, recorded as a request — never an
                // AppEvent, never a silent drop in replay.
                if let Some(text) = self.osc52.take() {
                    if self.tape.request("terminal.osc52", &text)? {
                        self.terminal_output.push(text);
                    }
                }
            }
            Action::Frame { columns, rows } => {
                if u32::from(columns) * u32::from(rows) > 1_000_000 {
                    return Err(io::Error::other("invalid frame dimensions"));
                }
                if !self.should_quit && !self.docs.is_empty() {
                    // Renders through the shared draw: in replay that draw
                    // consumes the recorded cell observation.
                    crate::headless::render_frame(self, columns, rows, false)?;
                }
            }
            Action::Finish => {
                self.finish_background_work();
            }
        }
        self.tape.healthy()?;
        if !frame && self.tape.observes() {
            // Frames check the canonical cell grid instead; every other
            // action checks the full logical observation.
            self.tape.check(&self.observation())?;
        }
        Ok(())
    }

    /// The logical observation both modes must reproduce bit-for-bit.
    /// Document content arrives as the pure `BufferSeed` (text, revision,
    /// history, disk baseline) — diagnostic trace identities and other
    /// process-local ephemera are deliberately absent.
    fn observation(&self) -> serde_json::Value {
        let documents: Vec<_> = self
            .docs
            .iter()
            .map(
                |(id, document)| serde_json::json!({"document": id, "buffer": document.buf.seed()}),
            )
            .collect();
        serde_json::json!({
            "documents": documents,
            "panes": self.panes,
            "mru": self.mru,
            "active": self.active_pane,
            "mode": self.mode.chip(),
            "quit": self.should_quit,
            "message": self.message,
            "headless": crate::headless::state_json(self),
            "hover": self.hover_card,
            "hunks": self.hunks,
            "staged_hunks": self.staged_hunks,
            "picker_items": self.picker.as_ref().map(|glue| &glue.picker.items),
        })
    }
}

/// Replay a recorded forensic node stream end to end. The seed is taken
/// first, the pure editor is reconstructed, and every recorded action is
/// consumed through the shared body until the deliberate `End`; any
/// divergence, gap or unexpected residue is an error, never a warning.
pub fn replay(nodes: Vec<strop_trace::replay::Node>) -> io::Result<Editor> {
    let tape = std::rc::Rc::new(strop_trace::replay::Tape::replay(nodes));
    let seed: super::seed::Seed = tape.take_seed()?;
    let mut editor = seed.into_editor(tape)?;
    while let Some(action) = editor.tape.next::<Action>()? {
        editor.apply_recorded(action)?;
    }
    editor.tape.healthy()?;
    Ok(editor)
}
