use super::*;
use crate::editor::{Document, DocumentSource, Mode};
use strop_core::Buffer;
use strop_terminal::{
    launch::Launch,
    model::{Effect, Palette, Update, MAX_COLUMNS, MAX_ROWS, MAX_SESSIONS},
};

impl Editor {
    pub(crate) fn launch_terminal(&mut self, command: &str, local: bool) {
        let location = if local {
            strop_workspace::ResourceLocation::local(self.cwd.clone())
        } else {
            self.directory_context()
        };
        self.launch_terminal_at(location, command);
    }

    pub(crate) fn launch_terminal_at(
        &mut self,
        location: strop_workspace::ResourceLocation,
        command: &str,
    ) {
        if location.local_path().is_none() {
            self.message = "interactive terminals are local only; use :terminal-local for an explicit local shell".into();
            return;
        }
        if self.terminals.entries.len() >= MAX_SESSIONS {
            self.message = format!(
                "terminal buffer limit reached ({MAX_SESSIONS}); close an exited terminal first"
            );
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let keyboard = self.terminals.keyboard;
        if !self.terminals.capture {
            if let Err(error) = self.tape.omit_content("terminal") {
                self.message = error.to_string();
                return;
            }
        }
        let session = SessionId::from_request(request);
        let geometry = Geometry {
            columns: 80,
            rows: (self.view_rows.max(1).min(MAX_ROWS as usize)) as u16,
            revision: 1,
        };
        let directory = location.path;
        let mut buffer = Buffer::from_text("");
        buffer.name = Some(format!("terminal #{}", session.get()));
        let mut document = Document::output(buffer);
        // Entering a terminal is a jump: ctrl-o returns to the editing
        // position the way any other buffer landing does.
        document.set_return_point(self.jump_record());
        document.source = DocumentSource::Terminal(Box::new(TerminalDocument {
            session,
            frame: None,
        }));
        let Ok(id) = self.docs.try_insert(document) else {
            self.message = "document identity space exhausted".into();
            return;
        };
        self.terminals.entries.insert(
            session,
            Entry {
                document: id,
                phase: Phase::Starting,
                live: None,
                service: None,
                directory: directory.clone(),
                title: None,
                geometry,
                paste: None,
                keyboard,
            },
        );
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.mode = Mode::Normal;
        self.view_mut().terminal_input = true;
        let sender = self.terminals.tx.clone();
        let tape = self.tape.clone();
        let result: Result<(), String> = match tape.call(
            "terminal.start",
            &(session, &directory, command, geometry, keyboard),
            || {
                let launch = Launch::shell(
                    directory.clone(),
                    (!command.is_empty()).then(|| command.into()),
                );
                Service::start(
                    session,
                    launch,
                    geometry,
                    keyboard,
                    Some(Palette::strop()),
                    move |session| {
                        let _ = sender.send(session);
                    },
                )
                .map(|service| {
                    if let Some(entry) = self.terminals.entries.get_mut(&session) {
                        entry.service = Some(service);
                    }
                })
                .map_err(|error| error.to_string())
            },
        ) {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            if let Some(entry) = self.terminals.entries.get_mut(&session) {
                entry.phase = Phase::Failed(error.clone());
            }
            self.view_mut().terminal_input = false;
            self.message = error;
        } else {
            self.message.clear();
        }
    }

    pub fn handle_terminal_update(&mut self, session: SessionId) {
        if !self.terminals.entries.contains_key(&session) {
            return;
        }
        let tape = self.tape.clone();
        let result = tape.call("terminal.update", &session, || {
            self.terminals
                .entries
                .get(&session)
                .and_then(|entry| entry.service.as_ref())
                .and_then(Service::take_update)
        });
        match result {
            Ok(Some(update)) => {
                if self.terminals.capture {
                    self.apply_terminal_update(update);
                } else {
                    strop_trace::without_content(|| self.apply_terminal_update(update));
                }
            }
            Ok(None) => {}
            Err(error) => self.message = error.to_string(),
        }
    }

    pub(super) fn apply_terminal_update(&mut self, update: Update) {
        let current = self.current();
        let Some(entry) = self.terminals.entries.get_mut(&update.session) else {
            return;
        };
        let document = entry.document;
        if let Some(frame) = update.frame {
            if frame.session != update.session
                || entry
                    .live
                    .as_ref()
                    .is_some_and(|old| frame.revision <= old.revision)
            {
                self.message = "terminal publication has stale or foreign identity".into();
                return;
            }
            entry.live = Some(frame);
        }
        if entry.phase != Phase::Closing || update.phase != Phase::Running {
            entry.phase = update.phase;
        }
        for effect in update.effects {
            match effect {
                Effect::Title(title) => entry.title = Some(title),
                Effect::PasteConfirmationRequested { ticket } => entry.paste = Some(ticket),
                Effect::PasteConfirmationCleared { ticket } => {
                    if entry.paste == Some(ticket) {
                        entry.paste = None;
                    }
                }
                // Reported cwd is untrusted metadata, not an editor context change.
                Effect::ReportedDirectory(_) | Effect::Bell => {}
                Effect::ClipboardWriteDenied | Effect::HostControlDenied => {
                    if current == document {
                        self.message = "terminal host-control request denied".into();
                    }
                }
            }
        }
        if !entry.phase.live() {
            entry.paste = None;
            entry.service = None;
        }
        let phase = entry.phase.clone();
        if self.docs.get(document).is_none() {
            return;
        }
        let inspecting = self
            .panes
            .iter()
            .any(|pane| pane.doc == document && !pane.terminal_input);
        let initial = self
            .terminal_document(document)
            .is_some_and(|source| source.frame.is_none());
        if initial || !inspecting {
            self.install_terminal_frame(document);
        }
        if !phase.live() {
            for pane in &mut self.panes {
                if pane.doc == document {
                    pane.terminal_input = false;
                }
            }
            if self.current() == document {
                self.mode = Mode::Normal;
                self.message = match phase {
                    Phase::Exited {
                        signal: Some(signal),
                        ..
                    } => format!("terminal ended by signal {signal}; output retained"),
                    Phase::Exited {
                        code: Some(code), ..
                    } => format!("terminal exited: {code}; output retained"),
                    Phase::Exited { .. } => "terminal closed before program startup".into(),
                    Phase::Failed(message) => message,
                    _ => unreachable!("non-live terminal phase"),
                };
            }
        } else if self.current() == document {
            if let Some(warning) = update.warning {
                self.message = warning;
            }
        }
    }

    pub(super) fn install_terminal_frame(&mut self, document: DocumentId) {
        let Some(source) = self.terminal_document(document) else {
            return;
        };
        let old_origin = source.frame.as_ref().map_or(0, |frame| frame.origin);
        let Some(frame) = self
            .terminals
            .entries
            .get(&source.session)
            .and_then(|entry| entry.live.clone())
        else {
            return;
        };
        let origin = frame.origin;
        let length = frame.projection.len_bytes();
        let result =
            self.doc_mut(document)
                .replace_snapshot(frame.projection.clone(), |position| {
                    old_origin
                        .saturating_add(position as u64)
                        .saturating_sub(origin)
                        .min(length as u64) as usize
                });
        if let Err(error) = result {
            self.message = format!("terminal projection failed: {error}");
            return;
        }
        if let DocumentSource::Terminal(source) = &mut self.doc_mut(document).source {
            source.frame = Some(frame.clone());
        }
        if self.current() == document && self.view().terminal_input {
            self.set_head(frame.cursor_byte());
        }
    }

    /// D2's explicit refresh: reinstall the latest published frame into an
    /// inspected (pinned) terminal. Explicit-only — output never drags a
    /// view — and the origin-based remap in `install_terminal_frame`
    /// carries cursor, selections and anchors to the same content.
    pub(crate) fn refresh_terminal(&mut self) {
        let document = self.current();
        let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        else {
            self.message = "current buffer is not a terminal".into();
            return;
        };
        if self.terminal_input_active() {
            self.message =
                "terminal input follows live output; Ctrl-\\ Ctrl-N inspects a snapshot".into();
            return;
        }
        if !self
            .terminals
            .entries
            .get(&session)
            .is_some_and(|entry| entry.live.is_some())
        {
            self.message = "terminal has no output yet".into();
            return;
        }
        let stale = self.terminal_has_new_output(document);
        if self.terminals.capture {
            self.install_terminal_frame(document);
        } else {
            strop_trace::without_content(|| self.install_terminal_frame(document));
        }
        self.message = if stale {
            "terminal view refreshed to the latest output".into()
        } else {
            "terminal view already shows the latest output".into()
        };
    }

    pub fn prepare_terminal_geometry(&mut self, document: DocumentId, columns: u16, rows: u16) {
        if self.current() != document || !self.terminal_input_active() || columns == 0 || rows == 0
        {
            return;
        }
        let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        else {
            return;
        };
        let Some(entry) = self.terminals.entries.get_mut(&session) else {
            return;
        };
        let (columns, rows) = (columns.min(MAX_COLUMNS), rows.min(MAX_ROWS));
        if (entry.geometry.columns, entry.geometry.rows) == (columns, rows) {
            return;
        }
        let Some(revision) = entry.geometry.revision.checked_add(1) else {
            self.message = "terminal geometry revision exhausted".into();
            return;
        };
        let geometry = Geometry {
            columns,
            rows,
            revision,
        };
        let tape = self.tape.clone();
        let result: Result<bool, String> =
            match tape.call("terminal.resize", &(session, geometry), || {
                let service = entry
                    .service
                    .as_ref()
                    .ok_or_else(|| "terminal service unavailable".to_owned())?;
                match service.resize(geometry) {
                    Ok(()) => Ok(true),
                    Err(strop_terminal::Error::InputFull) => Ok(false),
                    Err(error) => Err(error.to_string()),
                }
            }) {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
        match result {
            Ok(true) => entry.geometry = geometry,
            Ok(false) => {}
            Err(error) => self.message = format!("terminal resize: {error}"),
        }
    }

    pub(crate) fn stop_terminal(&mut self) {
        if let Some(session) = self
            .terminal_document(self.current())
            .map(|source| source.session)
        {
            self.stop_terminal_session(session);
        } else {
            self.message = "current buffer is not a terminal".into();
        }
    }
    fn stop_terminal_session(&mut self, session: SessionId) {
        let tape = self.tape.clone();
        if let Some(entry) = self.terminals.entries.get_mut(&session) {
            if !entry.phase.live() {
                return;
            }
            entry.phase = Phase::Closing;
            if let Err(error) = tape.call("terminal.stop", &session, || {
                if let Some(service) = &mut entry.service {
                    service.stop();
                }
            }) {
                self.message = error.to_string();
            }
        }
    }
    pub(crate) fn stop_all_terminals(&mut self) {
        let mut sessions: Vec<_> = self.terminals.entries.keys().copied().collect();
        sessions.sort_unstable_by_key(|session| session.get());
        for session in sessions {
            self.stop_terminal_session(session);
        }
    }
    pub(crate) fn forget_terminal_document(&mut self, document: DocumentId) {
        if let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        {
            debug_assert!(!self
                .terminals
                .entries
                .get(&session)
                .is_some_and(|entry| entry.phase.live()));
            self.terminals.entries.remove(&session);
        }
    }
    pub fn drain_terminals(&mut self) {
        for _ in 0..super::super::events::EVENTS_PER_TURN {
            let session = self
                .terminals
                .rx
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            let Some(session) = session else { break };
            self.handle_terminal_update(session);
        }
    }
}
