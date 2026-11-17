//! The picker's generation guard, explored exhaustively (0108 D3).
//!
//! A search source streams rows and a terminal onto the event loop; by the
//! time they are handled the user may have typed a new query, closed the
//! picker or reopened it. `handle_picker_event` admits an event only from
//! the generation that currently owns the stream. This test runs **every**
//! sequence of those events up to a fixed length against the real editor —
//! the real query restart, `close_picker` and `open_search` — and checks
//! after each step that the rows the picker holds are exactly the rows its
//! current generation delivered while it was live, and that the stream's
//! owner and streaming flag agree.
//!
//! The editor's tape is a fixture, so a query registers its generation
//! and launches no process; deliveries are injected the way a source
//! posts them. Ranking is suppressed with it, so the check is on the
//! item catalog — the set every shown row is drawn from. One editor
//! serves every sequence: each starts from a closed picker with nothing
//! retained, which is what a fresh editor has.

use super::*;
use std::rc::Rc;
use strop_core::worker::Outcome;
use strop_core::Buffer;
use strop_trace::replay::Tape;

#[derive(Debug, Clone, Copy)]
enum Step {
    Type,
    PartialLatest,
    PartialPrevious,
    FinishLatest,
    FinishPrevious,
    /// Close the picker if it is open, reopen it if it is closed (closing
    /// a closed picker or reopening an open one changes nothing).
    CloseOrReopen,
}

const STEPS: [Step; 6] = [
    Step::Type,
    Step::PartialLatest,
    Step::PartialPrevious,
    Step::FinishLatest,
    Step::FinishPrevious,
    Step::CloseOrReopen,
];

const LENGTH: usize = 5;
const PROBE: &str = "probe-";

/// What a correct guard leaves behind.
#[derive(Default)]
struct Model {
    open: bool,
    /// The generation that owns the stream, if it is still streaming.
    live: Option<Ticket<PickerKey>>,
    /// Rows delivered by the current generation while it was live.
    rows: Vec<String>,
}

/// Every generation the sequence has seen, oldest first; deliveries pick
/// the newest or the one before it (any older one is just as retired).
struct Run<'a> {
    editor: &'a mut Editor,
    issued: Vec<Ticket<PickerKey>>,
    model: Model,
    delivered: usize,
    refused: usize,
}

fn editor(root: &std::path::Path) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text("x\n"), root.to_path_buf());
    editor.tape = Rc::new(Tape::fixture(|_, _| {
        Err(std::io::Error::other("native work forbidden"))
    }));
    editor
}

impl<'a> Run<'a> {
    fn start(editor: &'a mut Editor) -> Self {
        editor.close_picker();
        editor.retained_search = None;
        editor.open_search(false);
        let mut run = Self {
            editor,
            issued: Vec::new(),
            model: Model {
                open: true,
                ..Model::default()
            },
            delivered: 0,
            refused: 0,
        };
        // Two generations exist before the explored steps: the first
        // query and its successor.
        run.step(Step::Type);
        run.step(Step::Type);
        run
    }

    /// A query restart issued a generation: it must be new, it owns the
    /// stream, and the previous generation's rows are gone.
    fn issued_new_generation(&mut self) {
        let ticket = self
            .editor
            .picker
            .as_ref()
            .and_then(|glue| glue.active.clone())
            .expect("a query restart registers its generation");
        assert!(
            !self.issued.contains(&ticket),
            "a restart reused generation {ticket:?}"
        );
        self.issued.push(ticket.clone());
        self.model.live = Some(ticket);
        self.model.rows.clear();
    }

    fn pick(&self, back: usize) -> Ticket<PickerKey> {
        self.issued[self.issued.len() - 1 - back].clone()
    }

    fn deliver(&mut self, ticket: Ticket<PickerKey>, msg: PickerMsg) {
        let owns = self.model.open && self.model.live.as_ref() == Some(&ticket);
        if owns {
            self.delivered += 1;
        } else {
            self.refused += 1;
        }
        match &msg {
            PickerMsg::Items(items) if owns => self
                .model
                .rows
                .extend(items.iter().map(|item| item.text.clone())),
            PickerMsg::Finished(_) if owns => self.model.live = None,
            _ => {}
        }
        self.editor.handle_picker_event(PickerEvent { ticket, msg });
    }

    fn row(&self, ticket: &Ticket<PickerKey>) -> Item {
        let text = format!("{PROBE}{}-{}", ticket.request.get(), self.model.rows.len());
        Item {
            badge: None,
            text,
            facts: Default::default(),
            payload: Payload::File(PathBuf::from("probe.txt")),
        }
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Type => {
                if !self.model.open {
                    return;
                }
                self.editor.paste_bracketed("a");
                self.issued_new_generation();
            }
            Step::PartialLatest | Step::PartialPrevious => {
                let back = usize::from(matches!(step, Step::PartialPrevious));
                let ticket = self.pick(back);
                let row = self.row(&ticket);
                self.deliver(ticket, PickerMsg::Items(vec![row].into()));
            }
            Step::FinishLatest | Step::FinishPrevious => {
                let back = usize::from(matches!(step, Step::FinishPrevious));
                let ticket = self.pick(back);
                self.deliver(ticket, PickerMsg::Finished(Outcome::Success(())));
            }
            Step::CloseOrReopen if self.model.open => {
                self.editor.close_picker();
                self.model.open = false;
                self.model.live = None;
                self.model.rows.clear();
            }
            Step::CloseOrReopen => {
                // The retained search comes back and refreshes its query.
                self.editor.open_search(false);
                self.model.open = true;
                self.issued_new_generation();
            }
        }
    }

    fn check(&self, trace: &[Step]) {
        assert_eq!(
            self.editor.picker_open(),
            self.model.open,
            "picker presence after {trace:?}"
        );
        let Some(glue) = self.editor.picker.as_ref() else {
            return;
        };
        let rows: Vec<String> = glue
            .picker
            .items
            .iter()
            .filter(|item| item.text.starts_with(PROBE))
            .map(|item| item.text.clone())
            .collect();
        assert_eq!(
            rows, self.model.rows,
            "rows are exactly the live generation's deliveries after {trace:?}"
        );
        assert_eq!(glue.active, self.model.live, "stream owner after {trace:?}");
        assert_eq!(
            glue.picker.streaming,
            self.model.live.is_some(),
            "streaming flag after {trace:?}"
        );
    }
}

#[test]
fn every_sequence_publishes_only_the_live_generation() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = editor(root.path());
    let mut sequences = 0usize;
    let mut delivered = 0usize;
    let mut refused = 0usize;
    let mut trace = [Step::Type; LENGTH];
    let total = STEPS.len().pow(LENGTH as u32);
    for mut code in 0..total {
        for slot in trace.iter_mut() {
            *slot = STEPS[code % STEPS.len()];
            code /= STEPS.len();
        }
        let mut run = Run::start(&mut editor);
        run.check(&[]);
        for (index, &step) in trace.iter().enumerate() {
            run.step(step);
            run.check(&trace[..=index]);
        }
        sequences += 1;
        delivered += run.delivered;
        refused += run.refused;
    }
    assert_eq!(sequences, 7_776);
    // The space is not vacuous: both sides of the guard were exercised.
    assert!(delivered > 0 && refused > 0, "{delivered} / {refused}");
}
