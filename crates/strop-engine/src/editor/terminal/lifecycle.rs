use super::*;
use crate::editor::{Document, DocumentSource, Mode};
use strop_core::Buffer;
use strop_terminal::{
    launch::Launch,
    model::{Effect, Palette, Update, MAX_COLUMNS, MAX_ROWS, MAX_SESSIONS},
    service::Target,
};

impl Editor {
    /// Let each existing service update its native defaults to `theme` and
    /// publish through the ordinary replay-owned mailbox.
    pub(crate) fn set_terminal_themes(
        &mut self,
        theme: strop_core::theme::ThemeId,
    ) -> Result<(), String> {
        let tape = self.tape.clone();
        match tape.call("terminal.theme", &theme.name(), || {
            let mut result = Ok(());
            for entry in self.terminals.entries.values() {
                if !matches!(entry.phase, Phase::Starting | Phase::Running) {
                    continue;
                }
                if let Some(service) = &entry.service {
                    match service.set_theme(theme) {
                        Ok(()) | Err(strop_terminal::Error::Closed) => {}
                        Err(error) => result = Err(error.to_string()),
                    }
                }
            }
            result
        }) {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        }
    }

    /// `:terminal [cmd]` / `Space T`: a new session in the current pane.
    pub(crate) fn launch_terminal(&mut self, command: &str, local: bool) {
        let location = if local {
            strop_workspace::ResourceLocation::local(self.cwd.clone())
        } else {
            self.current_workspace_root()
        };
        self.launch_terminal_at(location, command);
    }

    /// `Space t`: a new session in a horizontal split below the current
    /// pane (0090 §4). The split opens on the current document, then the
    /// launch replaces it in that pane — one jump record, as `:terminal`.
    pub(crate) fn launch_terminal_split(&mut self) {
        let before = self.panes.len();
        self.split_document(false, self.current());
        self.launch_terminal("", false);
        if self.terminal_document(self.current()).is_none() && self.panes.len() > before {
            // The launch was refused: do not leave an empty split behind.
            self.close_pane_or_buffer(false);
        }
    }

    pub(crate) fn launch_terminal_at(
        &mut self,
        location: strop_workspace::ResourceLocation,
        command: &str,
    ) {
        let target = match terminal_target(self, &location) {
            Ok(target) => target,
            Err(message) => {
                self.message = message;
                return;
            }
        };
        if self.terminals.entries.len() >= MAX_SESSIONS {
            self.message = format!(
                "terminal buffer limit reached ({MAX_SESSIONS}); close an exited terminal first"
            );
            return;
        }
        let keyboard = self.terminals.keyboard;
        if !self.terminals.capture {
            if let Err(error) = self.tape.omit_content("terminal") {
                self.message = error.to_string();
                return;
            }
        }
        let Some(session) = self.terminals.next_session() else {
            self.message = "terminal session numbers exhausted".into();
            return;
        };
        let geometry = Geometry {
            columns: 80,
            rows: (self.view_rows.max(1).min(MAX_ROWS as usize)) as u16,
            revision: 1,
        };
        let directory = location.path.clone();
        let launch = match &location.filesystem {
            strop_workspace::Filesystem::Local => Launch::shell(
                directory.clone(),
                (!command.is_empty()).then(|| command.into()),
            ),
            // A remote namespace never receives the editor's
            // environment (0058 WK12 privacy): minimal identity only.
            _ => Launch::shell_remote(
                directory.clone(),
                (!command.is_empty()).then(|| command.into()),
            ),
        };
        let program = if command.is_empty() {
            match &location.filesystem {
                strop_workspace::Filesystem::Local => std::path::Path::new(&launch.program)
                    .file_name()
                    .map_or_else(
                        || launch.program.to_string_lossy().into_owned(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                _ => "sh".to_owned(),
            }
        } else {
            command.to_owned()
        };
        let mut buffer = Buffer::from_text("");
        buffer.name = Some(terminal_name(session, &location, &program));
        let mut document = Document::output(buffer);
        // Entering a terminal is a jump: ctrl-o returns to the editing
        // position the way any other buffer landing does.
        document.set_return_point(self.jump_record());
        document.source = DocumentSource::Terminal(Box::new(TerminalDocument {
            session,
            launch: location.clone(),
            frame: None,
        }));
        let Ok(id) = self.docs.try_insert(document) else {
            self.message = "document identity space exhausted".into();
            return;
        };
        self.push_jump();
        self.terminals.entries.insert(
            session,
            Entry {
                document: id,
                phase: Phase::Starting,
                service: None,
                launch: location.clone(),
                title: None,
                geometry,
                keyboard,
                view: None,
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
            &(session, &location, command, geometry, keyboard),
            || {
                Service::start(
                    session,
                    target,
                    launch,
                    geometry,
                    keyboard,
                    Some(Palette::for_theme(self.presentation.theme)),
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
            self.sync_terminal_feedback(id, false);
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
        if entry.phase != Phase::Closing || update.phase != Phase::Running {
            entry.phase = update.phase;
        }
        let denied = update.effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::ClipboardWriteDenied | Effect::HostControlDenied
            )
        });
        for effect in update.effects {
            match effect {
                Effect::Title(title) => entry.title = Some(title),
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
            entry.service = None;
        }
        let phase = entry.phase.clone();
        if self.docs.get(document).is_none() {
            return;
        }
        let initial = self
            .terminal_document(document)
            .is_some_and(|source| source.frame.is_none());
        if let Some(frame) = update.frame {
            let stale = frame.session != update.session
                || self
                    .terminal_frame(document)
                    .is_some_and(|old| frame.revision <= old.revision);
            if stale {
                self.message = "terminal publication has stale or foreign identity".into();
                return;
            }
            self.install_terminal_frame(document, frame);
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
                    Phase::Exited { .. } if initial => {
                        "terminal closed before program startup".into()
                    }
                    Phase::Exited { .. } => "terminal ended; output retained".into(),
                    Phase::Failed(message) => message,
                    _ => unreachable!("non-live terminal phase"),
                };
            }
        } else if self.current() == document {
            if let Some(warning) = update.warning {
                self.message = warning;
            }
        }
        self.sync_terminal_feedback(document, denied);
    }

    /// Install a published frame as the buffer's text — the live buffer
    /// of 0090 §4. Positions map by stream byte, so a cursor, selection
    /// or mark stays on the same content when scrollback drops rows.
    /// Views then settle like Neovim's windows: a TERMINAL view sits on
    /// the child's cursor; a NORMAL view whose cursor was on the last
    /// line follows output to the new last line; any other NORMAL view
    /// stays where it was.
    fn install_terminal_frame(&mut self, document: DocumentId, frame: Arc<Frame>) {
        let old_origin = self
            .terminal_frame(document)
            .map_or(frame.origin, |old| old.origin);
        let following = {
            let buf = &self.doc(document).buf;
            let last = buf.last_content_line();
            self.panes
                .iter()
                .enumerate()
                .filter(|(_, pane)| pane.doc == document && !pane.terminal_input)
                .filter_map(|(index, pane)| {
                    let head = pane.sels.primary().head.min(buf.len_bytes());
                    (buf.line_of(head) >= last).then(|| (index, buf.col_of(head)))
                })
                .collect::<Vec<_>>()
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
        let cursor = frame.cursor_byte();
        if let DocumentSource::Terminal(source) = &mut self.doc_mut(document).source {
            source.frame = Some(frame);
        }
        let (last_start, last_end) = {
            let buf = &self.doc(document).buf;
            let last = buf.last_content_line();
            (buf.line_start(last), buf.line_end(last))
        };
        let active = self.active_pane;
        for (index, pane) in self.panes.iter_mut().enumerate() {
            if pane.doc != document {
                continue;
            }
            if pane.terminal_input {
                pane.sels.collapse_primary(cursor);
                pane.desired_column = None;
                continue;
            }
            if let Some((_, column)) = following.iter().find(|(follower, _)| *follower == index) {
                pane.sels
                    .set_head(last_start.saturating_add(*column).min(last_end));
                pane.desired_column = None;
                if index != active {
                    // The viewport clamp in `prepare` pulls this back to
                    // "last line at the bottom"; the active pane's own
                    // cursor scroll reveals its cursor instead.
                    pane.view_top = pane.view_top.max(last_start);
                }
            }
        }
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

    pub(crate) fn stop_terminal_session(&mut self, session: SessionId) {
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
    /// `:bd!` on a running terminal: stop the child and drop the session
    /// so the buffer can close like any other. The final update for a
    /// dropped session has nowhere to land and is ignored.
    pub(crate) fn kill_terminal_document(&mut self, document: DocumentId) {
        let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        else {
            return;
        };
        self.stop_terminal_session(session);
        self.resolve_terminal_session_feedback(session);
        self.terminals.entries.remove(&session);
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
            self.resolve_terminal_session_feedback(session);
            self.terminals.entries.remove(&session);
        }
    }

    /// Leaving a terminal buffer (0090 §4): keep its cursor and viewport
    /// as stream positions so re-entry lands where the user was, and
    /// keeps following output if they were at the bottom.
    pub(crate) fn remember_terminal_view(&mut self) {
        // A switch away from an already-removed document carries no view.
        let document = self.current();
        let Some(DocumentSource::Terminal(source)) =
            self.docs.get(document).map(|document| &document.source)
        else {
            return;
        };
        let session = source.session;
        let origin = source.frame.as_ref().map_or(0, |frame| frame.origin);
        let view = self.view();
        let buf = &self.doc(document).buf;
        let head = view.sels.primary().head.min(buf.len_bytes());
        let top = buf.line_start(view.view_top.min(buf.len_lines().saturating_sub(1)));
        let saved = SavedView {
            head: origin.saturating_add(head as u64),
            top: origin.saturating_add(top as u64),
            following: view.terminal_input || buf.line_of(head) >= buf.last_content_line(),
        };
        if let Some(entry) = self.terminals.entries.get_mut(&session) {
            entry.view = Some(saved);
        }
    }

    /// Arriving at a terminal buffer: NORMAL mode, cursor and viewport
    /// restored (a following view lands on the last line again).
    pub(crate) fn restore_terminal_view(&mut self, document: DocumentId) {
        let Some(source) = self.terminal_document(document) else {
            return;
        };
        let session = source.session;
        let origin = source.frame.as_ref().map_or(0, |frame| frame.origin);
        let Some(saved) = self
            .terminals
            .entries
            .get(&session)
            .and_then(|entry| entry.view)
        else {
            return;
        };
        let buf = &self.doc(document).buf;
        let map = |stream: u64| {
            buf.clamp_boundary(stream.saturating_sub(origin).min(buf.len_bytes() as u64) as usize)
        };
        let (head, top) = if saved.following {
            let last = buf.last_content_line();
            (buf.line_start(last), buf.line_start(last))
        } else {
            (map(saved.head), buf.line_of(map(saved.top)))
        };
        let view = self.view_mut();
        view.sels.collapse_primary(head);
        view.desired_column = None;
        view.view_top = top;
        view.terminal_input = false;
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

/// Where one terminal's PTY runs (0090 §4). A local directory spawns the
/// PTY in the editor process; an SSH workspace rides its already-admitted
/// worker lease (deploy is consent-gated and blocking, so a terminal never
/// triggers it from the input path — an endpoint without a live lease is
/// a typed refusal). No remote path ever falls back to the host.
fn terminal_target(
    editor: &Editor,
    location: &strop_workspace::ResourceLocation,
) -> Result<Target, String> {
    match &location.filesystem {
        strop_workspace::Filesystem::Local => Ok(Target::Local),
        strop_workspace::Filesystem::Remote(endpoint) => editor
            .remote
            .workers
            .get(endpoint)
            .map(|worker| Target::Worker(worker.worker().clone()))
            .ok_or_else(|| {
                format!(
                    "no admitted worker on {endpoint}: interactive terminals there need one \
                     (a worker-admitting remote action admits it); \
                     use :terminal-local for an explicit local shell"
                )
            }),
    }
}
