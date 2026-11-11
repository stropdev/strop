//! Registers and paste (vim's " machinery) plus the system clipboard
//! (`+` via OSC52/wl-paste, helix's playbook). Reads run on workers;
//! results land on the drain — never a subprocess on the input path.
//!
//! Register CONTENT is a typed domain (R13): [`Register`] carries its
//! text plus a [`RegisterShape`] — the `(String, bool)` tuple could
//! not say blockwise, and a block needs its rectangle width in
//! display cells to land straight under configured tabs and wide
//! clusters. No bare `bool` crosses the register or paste boundary.

mod clipboard;
mod put;
mod store;

use strop_core::id::DisplayColumn;
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket};

use super::{trace, Editor};
pub(crate) use clipboard::PasteDestination;
use clipboard::{field_text, read_system_clipboard};
pub use store::RegisterError;
pub(crate) use store::{Origin, RegisterStore};

/// The document that asked for a clipboard read (R9): the terminal
/// result owns this ticket — a paste lands only in the buffer that
/// requested it, and failures surface instead of collapsing into
/// "empty clipboard".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardKey {
    pub document: strop_core::id::DocumentId,
}

/// The owned terminal clipboard-read result.
pub type ClipboardResult = Completion<ClipboardKey, String>;

/// How register text sits in the document when it lands — vim's
/// three register shapes. Blockwise carries the rectangle's cell
/// width, measured with the configured tab size at yank time, so
/// paste replays the same rectangle on any tab/wide-char mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterShape {
    Characterwise,
    Linewise,
    /// Rows joined by `\n` in [`Register::text`]; `width` is the
    /// rectangle's width in display cells.
    Blockwise {
        width: DisplayColumn,
    },
}

/// One register cell: text + shape (vim's unnamed register is `"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub shape: RegisterShape,
    pub file_provenance: Option<std::sync::Arc<super::filesystem::draft::FileRegister>>,
}

impl Register {
    pub fn characterwise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shape: RegisterShape::Characterwise,
            file_provenance: None,
        }
    }

    pub fn linewise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shape: RegisterShape::Linewise,
            file_provenance: None,
        }
    }

    /// `width` is the rectangle's width in display cells (the same
    /// streamed layout that measured it at yank).
    pub fn blockwise(text: impl Into<String>, width: DisplayColumn) -> Self {
        Self {
            text: text.into(),
            shape: RegisterShape::Blockwise { width },
            file_provenance: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// The largest plain yank mirrored to the system clipboard. OSC52 is
/// base64 through the terminal; terminals cap or stall well below this.
const CLIPBOARD_MIRROR_MAX: usize = 1 << 20;

/// What one register read yields — THE read path (plan 0092 §1):
/// `p`/`P`, insert `Ctrl-R`, and every prompt and picker field ask
/// here, so a name means the same text on every surface.
pub(crate) enum RegisterRead {
    Text(Register),
    /// `"+`/`"*`: the live system clipboard is a provider job; the
    /// caller names where the text lands when it arrives.
    Clipboard,
}

impl Editor {
    /// The register's current content. Total: an invalid name reads
    /// empty, and `+`/`*` read what this editor last wrote there (the
    /// live clipboard is asynchronous — see [`RegisterRead`]).
    pub fn register(&self, name: Option<char>) -> Register {
        match self.read_register(name) {
            Ok(RegisterRead::Text(register)) => register,
            Ok(RegisterRead::Clipboard) => self
                .registers
                .get(Some('+'))
                .cloned()
                .unwrap_or_else(|_| Register::characterwise("")),
            Err(_) => Register::characterwise(""),
        }
    }

    pub(crate) fn read_register(&self, name: Option<char>) -> Result<RegisterRead, RegisterError> {
        let computed = |text: Option<String>| {
            Ok(RegisterRead::Text(Register::characterwise(
                text.unwrap_or_default(),
            )))
        };
        match name {
            Some('+' | '*') => Ok(RegisterRead::Clipboard),
            // vim's read-only registers are views of editor state
            Some('.') => computed(self.last_insert.clone()),
            Some(':') => computed(self.registers.command_line.clone()),
            Some('/') => computed(
                self.last_search
                    .as_ref()
                    .map(|search| search.query.source().to_owned()),
            ),
            Some('%') => computed(
                self.buf()
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            ),
            name => self
                .registers
                .get(name)
                .map(|register| RegisterRead::Text(register.clone())),
        }
    }

    /// A register name an operator may write (`"_` included). Refusing
    /// before the edit keeps a `".dd` from deleting text it cannot keep.
    pub(crate) fn register_writable(&mut self, name: Option<char>) -> bool {
        match RegisterStore::writable(name) {
            Ok(()) => true,
            Err(error) => {
                self.message = error.to_string();
                false
            }
        }
    }

    /// `y`: the text lands in `name` (the unnamed yank fills `"0`).
    pub(crate) fn yank_register(&mut self, name: Option<char>, register: Register) {
        self.write_register(name, register, Origin::Yank);
    }

    /// `d`/`c`/`x`: the numbered shift and the small-delete register
    /// apply. `reg_one` is vim's `use_reg_one` (see [`reg_one_motion`]).
    pub(crate) fn delete_register(
        &mut self,
        name: Option<char>,
        register: Register,
        reg_one: bool,
    ) {
        self.write_register(name, register, Origin::Delete { reg_one });
    }

    fn write_register(&mut self, name: Option<char>, register: Register, origin: Origin) {
        match self.registers.write(name, register, origin) {
            // A plain yank is a copy (0092 §1.4), but only an explicit
            // `"+y` may push megabytes through the terminal.
            Ok(Some(clipboard))
                if !matches!(name, Some('+' | '*')) && clipboard.len() > CLIPBOARD_MIRROR_MAX =>
            {
                self.message = format!(
                    "yanked {} bytes — too large to copy to the system clipboard (\"+y forces it)",
                    clipboard.len()
                );
            }
            // `+`/`*` are the system clipboard: the write stages an
            // OSC52 payload for the terminal adapter to emit
            Ok(Some(clipboard)) => self.osc52 = Some(clipboard),
            Ok(None) => {}
            Err(error) => self.message = error.to_string(),
        }
        // the register is no longer the recording: `@a` replays what
        // `"a` now holds (vim executes register content)
        if let Some(name) = name.filter(char::is_ascii_alphabetic) {
            self.macros.remove(&name.to_ascii_lowercase());
        }
    }

    /// `p`/`P` (vim `2p`): the register lands per its shape —
    /// charwise/linewise text repeats, a block repeats its rectangle
    /// horizontally; always one undo unit.
    pub(crate) fn paste(&mut self, name: Option<char>, count: usize, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        self.paste_from(name, PasteDestination::Put { before, count });
    }

    /// THE register read (plan 0092 §1): resolve `name`, then deliver.
    /// `+`/`*` resolve on a provider job and deliver on its result.
    pub(crate) fn paste_from(&mut self, name: Option<char>, destination: PasteDestination) {
        match self.read_register(name) {
            Ok(RegisterRead::Text(register)) => self.deliver_paste(destination, register),
            Ok(RegisterRead::Clipboard) => self.clipboard_read(destination),
            Err(error) => self.message = error.to_string(),
        }
    }

    fn deliver_paste(&mut self, destination: PasteDestination, register: Register) {
        if register.is_empty() {
            return;
        }
        match destination {
            PasteDestination::Put { before, count } => {
                self.paste_register(&register, count, before)
            }
            PasteDestination::Insert => self.insert_literal(
                &register.text,
                crate::editor::insert::replay::InsertTargets::All,
            ),
            PasteDestination::Prompt => self.feed_pending_event(
                crate::editor::pending::PendingEvent::Paste(field_text(&register).to_owned()),
            ),
            PasteDestination::Picker => self.paste_picker(field_text(&register)),
        }
    }

    /// The destination a read was requested for still has focus.
    fn paste_destination_live(&self, destination: PasteDestination) -> bool {
        match destination {
            PasteDestination::Put { .. } => true,
            PasteDestination::Insert => self.mode == crate::editor::Mode::Insert,
            PasteDestination::Prompt => self.pending.is_active(),
            PasteDestination::Picker => self.picker_open(),
        }
    }

    /// `Space p` / `"+p` / `Ctrl-R +`: spawn a clipboard read; the
    /// result lands in `handle_clipboard` on a later tick.
    pub(crate) fn clipboard_read(&mut self, destination: PasteDestination) {
        if self.clip_paste_pending.is_some() {
            return; // one read in flight
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: ClipboardKey {
                document: self.current(),
            },
        };
        self.clip_paste_pending = Some((destination, ticket.clone()));
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({"service":"clipboard","request":request.get(),
                "document":{"slot":self.current().index(),"generation":self.current().generation()}})
        });
        match self.tape.request("clipboard.read", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_clipboard(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let tx = self.clip_tx.clone();
        let handle = worker::spawn(
            "strop-clipboard",
            move |outcome| {
                let _ = tx.send(Completion { ticket, outcome });
            },
            |_| read_system_clipboard(),
        );
        self.worker_handles.insert(request, handle);
    }

    /// One owned clipboard-read result.
    pub(crate) fn handle_clipboard(&mut self, result: ClipboardResult) {
        trace::services::clipboard(&result);
        let Some((destination, ticket)) = self.clip_paste_pending.as_ref() else {
            return;
        };
        if *ticket != result.ticket {
            trace::services::rejected("clipboard", "clipboard request superseded");
            return;
        }
        let destination = *destination;
        self.clip_paste_pending = None;
        self.worker_handles.remove(&result.ticket.request);
        // the read answers the document and surface that asked (0023
        // §4): a switch mid-read must not paste into the newly active
        // buffer, nor a closed prompt's text into the document
        if self.docs.is_empty()
            || self.current() != result.ticket.key.document
            || !self.paste_destination_live(destination)
        {
            self.message = "clipboard: destination changed — paste dropped".into();
            trace::services::rejected("clipboard", &self.message);
            return;
        }
        match result.outcome {
            // a trailing newline is vim's linewise paste convention;
            // Success("") is a real empty clipboard, not a failure
            Outcome::Success(text) if !text.is_empty() => {
                let register = if text.ends_with('\n') {
                    Register::linewise(text)
                } else {
                    Register::characterwise(text)
                };
                self.deliver_paste(destination, register);
            }
            Outcome::Success(_) => self.message = "clipboard: empty".into(),
            Outcome::Failed { failure, .. } => {
                self.message = format!("clipboard: {}", failure.message)
            }
            // the owning paste left (cancelled/superseded): nothing to do
            Outcome::Cancelled(_) => {}
        }
    }

    /// Table shims (0008 stage 2): `Space y` arms the `+` register for
    /// the next yank; `Space p/P` paste from the system clipboard.
    pub(crate) fn clipboard_yank_pub(&mut self) {
        self.walker
            .begin_operator(strop_grammar::Op::Yank, Some('+'), None);
    }
    pub(crate) fn clipboard_paste_pub(&mut self, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        self.clipboard_read(PasteDestination::Put { before, count: 1 });
    }
    /// Bracketed paste (0017): one undo unit, no key interpretation —
    /// the payload is text, not keystrokes. A live prompt consumes it
    /// through the pending reducer; an open picker pastes into its
    /// focused field; in normal mode it behaves like p; a trailing
    /// newline pastes linewise (vim's paste plugin convention).
    pub fn paste_bracketed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.pending.is_active() {
            self.feed_pending_event(super::pending::PendingEvent::Paste(text.to_owned()));
            return;
        }
        if self.picker_open() {
            self.paste_picker(text);
            return;
        }
        if self.mode == super::Mode::Insert {
            self.insert_literal(text, super::insert::replay::InsertTargets::Primary);
        } else {
            let register = if text.ends_with('\n') {
                Register::linewise(text)
            } else {
                Register::characterwise(text)
            };
            self.paste_register(&register, 1, false);
        }
    }
}

/// vim's `use_reg_one`: a delete over one of these motions always
/// fills `"1`, even within a line (`:help quote_number`).
pub(crate) fn reg_one_motion(target: &strop_grammar::Target) -> bool {
    use strop_grammar::Motion;
    matches!(
        target,
        strop_grammar::Target::Motion(
            Motion::MatchPair
                | Motion::Search(_)
                | Motion::SearchBackward(_)
                | Motion::ParagraphForward
                | Motion::ParagraphBackward
        )
    )
}
