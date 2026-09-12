//! Local-channel delivery for tests; live drivers use the shared AppEvent channel.
use super::picker::PickerEvent;
use super::{trace, Editor};
use std::sync::mpsc::TryRecvError;
use strop_core::worker::{FailureKind, Outcome};
use strop_picker::PickerMsg;

impl Editor {
    /// A named local repository for pure surface fixtures; no discovery or I/O.
    pub fn fixture_git_context(&mut self) {
        self.git = Some(strop_git::GitContext {
            repo: strop_git::RepoTarget::Local {
                workdir: self.cwd.clone(),
            },
            head_sha: None,
            head_branch: None,
            remotes: Vec::new(),
        });
    }

    /// Local-channel fixture barrier; live drivers never call this.
    pub fn wait_io(&mut self) -> Result<(), String> {
        while self.io_pending() {
            let event = self
                .io
                .rx
                .as_ref()
                .ok_or("IO channel is forwarded")?
                .recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|error| error.to_string())?;
            self.handle_io(event);
        }
        Ok(())
    }

    pub fn wait_picker(&mut self) {
        while self
            .picker
            .as_ref()
            .is_some_and(|glue| glue.picker.streaming)
        {
            let (ticket, receiver) = self
                .picker
                .as_ref()
                .unwrap()
                .rx
                .as_ref()
                .expect("fixture retains the picker channel");
            let event = PickerEvent {
                ticket: ticket.clone(),
                msg: receiver
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap(),
            };
            self.handle_picker_event(event);
        }
        while self
            .picker
            .as_ref()
            .is_some_and(|glue| glue.rank_pending.is_some())
        {
            let event = self
                .picker_ranking
                .rx
                .as_ref()
                .expect("local ranking channel")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("ranking settled");
            self.handle_picker_ranking(event);
        }
    }

    /// Publish fixture candidates through the real owned ranking actor.
    pub fn picker_items_fixture(&mut self, items: Vec<strop_picker::Item>) {
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.append(items);
        }
        self.request_picker_ranking();
        self.wait_picker();
    }

    pub fn wait_analysis(&mut self) {
        while self.analysis.pending() {
            let event = self
                .analysis
                .rx
                .as_ref()
                .expect("local analysis channel")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("analysis settles");
            self.handle_analysis(event);
        }
    }

    pub fn drain_lsp(&mut self) {
        let mut events = Vec::new();
        for server in &self.lsp_servers {
            events.extend(server.rx.try_iter());
        }
        for event in events {
            self.handle_lsp_event(event);
        }
        while let Ok(record) = self.lsp_state.attach.rx.try_recv() {
            self.handle_lsp_attach(record);
        }
    }

    /// Collect clipboard reads (headless settle; the TUI forwards each
    /// result as an AppEvent the moment it lands — 0018). Empty and
    /// disconnected are distinct; an empty arena never strands the
    /// pending request.
    pub fn drain_clipboard(&mut self) {
        use std::sync::mpsc::TryRecvError;
        while let Some(rx) = self.clip_rx.as_ref() {
            match rx.try_recv() {
                Ok(result) => self.handle_clipboard(result),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    // ---- job drain ------------------------------------------------------

    pub fn drain_git_jobs(&mut self) {
        loop {
            let next = self.git_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            match next {
                Some(job) => {
                    self.handle_git_job(job);
                }
                None => break,
            }
        }
    }

    /// Drain worker messages (called from the event loop each tick).
    pub fn drain_picker(&mut self) {
        loop {
            // the active request's stream, headless-only (the TUI
            // bridges streams at launch)
            let next = self.picker.as_ref().and_then(|glue| {
                glue.rx
                    .as_ref()
                    .map(|(ticket, rx)| (ticket.clone(), rx.try_recv()))
            });
            let Some((ticket, received)) = next else {
                break;
            };
            match received {
                Ok(msg) => self.handle_picker_event(PickerEvent { ticket, msg }),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // channel died before any terminal event — detach,
                    // then let the handler reject or settle it
                    if let Some(glue) = self.picker.as_mut() {
                        glue.rx = None;
                    }
                    self.handle_picker_event(PickerEvent {
                        ticket,
                        msg: PickerMsg::Finished(Outcome::failed(
                            FailureKind::Disconnected,
                            "picker worker channel closed",
                        )),
                    });
                }
            }
        }
        while let Some(event) = self
            .picker_ranking
            .rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.handle_picker_ranking(event);
        }
        self.drain_previews();
    }

    /// Drain preview worker results (file reads happen off the render
    /// path — 0001 §3). Headless path; the TUI forwards each result as
    /// an AppEvent (0018).
    fn drain_previews(&mut self) {
        loop {
            let received = self.preview_rx.as_ref().map(|rx| rx.try_recv());
            match received {
                Some(Ok(result)) => self.handle_preview(result),
                Some(Err(TryRecvError::Empty)) | None => return,
                Some(Err(TryRecvError::Disconnected)) => {
                    // the editor keeps a sender: a disconnect is a
                    // wiring bug — surface it and stop draining
                    trace::services::rejected("preview", "preview channel disconnected");
                    self.preview_rx = None;
                    return;
                }
            }
        }
    }
}

/// Git-surface fixtures shared by engine and binary (frame) tests: real
/// on-disk repositories, real discovery, real job pumping.
pub mod git {
    use super::super::{Editor, Surface};
    use std::process::Command;
    use strop_core::Buffer;

    /// Repo with two commits; second adds a line to f.rs.
    pub fn fixture() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "first"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        git(&["commit", "-qam", "add b"]);
        let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.discover_git();
        settle(&mut e, |editor| editor.git.is_some());
        (dir, e)
    }

    /// Two files in one fixture repo; the second commit touches both.
    pub fn multi_file_fixture() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.rs"), "one\n").unwrap();
        std::fs::write(root.join("b.rs"), "uno\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(root.join("a.rs"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("b.rs"), "uno\ndos\n").unwrap();
        git(&["commit", "-qam", "touch both"]);
        let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
        e.cwd = root.to_path_buf();
        e.discover_git();
        settle(&mut e, |editor| editor.git.is_some());
        (dir, e)
    }

    /// Drain async work until `done` or the deadline (real threads need
    /// real pumping — the event loop's shape).
    pub fn settle(e: &mut Editor, done: impl Fn(&Editor) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(e) {
            if e.io_pending() {
                e.wait_io().unwrap();
            }
            e.drain_git_jobs();
            if done(e) {
                break;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = e
                .git_rx
                .as_ref()
                .unwrap()
                .recv_timeout(remaining)
                .expect("git job completes");
            e.handle_git_job(event);
        }
    }

    /// Settle discovery, then the open log surface's rows.
    pub fn pump(e: &mut Editor) {
        settle(e, |e| e.git.is_some());
        settle(e, |e| {
            matches!(
                e.surface(),
                Some(Surface::CommitLog { rows, .. }) if !rows.is_empty()
            )
        });
    }

    pub fn pump_ready(e: &mut Editor, ready: impl Fn(&Editor) -> bool) {
        settle(e, ready);
    }

    pub fn git_out(root: &std::path::Path, args: &[&str]) -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string()
    }
}

/// Remote-snapshot fixtures: hermetic remote buffers, never a client.
pub mod remote {
    use super::super::document::RemoteDocument;
    use super::super::io::{IoEvent, OpenKey, Opened};
    use super::super::{Document, Editor};
    use crate::files::FileTarget;
    use std::rc::Rc;
    use strop_core::worker::{Completion, Outcome, Ticket};
    use strop_core::Buffer;
    use strop_remote::ReadSelection;
    use strop_trace::replay::Tape;
    use strop_workspace::RemoteFile;

    /// An isolated editor whose tape answers analysis start and forbids
    /// every other native observation.
    pub fn io_editor(text: &str) -> Editor {
        let mut editor = Editor::new_in(Buffer::from_text(text), "/isolated".into());
        editor.tape = Rc::new(Tape::fixture(|operation, _| match operation {
            "analysis.start" => Ok(serde_json::json!({"Ok": null})),
            _ => Err(std::io::Error::other("native work forbidden")),
        }));
        editor
    }

    /// An isolated editor showing one remote snapshot document.
    pub fn snapshot_editor(text: &str) -> Editor {
        let mut editor = Editor::new_in(Buffer::from_text("keepalive\n"), "/isolated".into());
        editor.tape = Rc::new(Tape::fixture(|_, _| {
            Err(std::io::Error::other("native work forbidden"))
        }));
        let id = editor.docs.insert(document(text, ReadSelection::Full));
        editor.switch_to(id);
        editor
    }

    pub fn document(text: &str, selection: ReadSelection) -> Document {
        Document::remote(
            Buffer::from_text(text),
            RemoteDocument {
                file: RemoteFile::parse("ssh://fixture/repo/app.log").unwrap(),
                window: strop_remote::RemoteWindow::resolve(
                    &selection,
                    strop_remote::RemoteSize::new(text.len() as u64),
                ),
                selection,
                connection: None,
                return_to: None,
                write: None,
            },
        )
    }

    pub fn ticket(editor: &Editor) -> Ticket<OpenKey> {
        let (&request, key) = editor.io.open.iter().next().unwrap();
        Ticket {
            request,
            key: key.clone(),
        }
    }

    pub fn deliver(editor: &mut Editor, ticket: Ticket<OpenKey>, text: &str) {
        let FileTarget::Remote(file) = &ticket.key.path else {
            panic!("remote ticket")
        };
        let result = Opened {
            document: Document::remote(
                Buffer::from_text(text),
                RemoteDocument {
                    file: file.absolute_file().unwrap().clone(),
                    window: strop_remote::RemoteWindow::resolve(
                        &ticket.key.selection,
                        strop_remote::RemoteSize::new(text.len() as u64),
                    ),
                    selection: ticket.key.selection,
                    connection: None,
                    return_to: None,
                    write: None,
                },
            ),
            canonical: ticket.key.path.clone(),
        };
        editor.handle_io(IoEvent::Open(Box::new(Completion {
            ticket,
            outcome: Outcome::Success(result),
        })));
    }

    /// The scripted `:e ssh://…` open, completed from the fixture.
    pub fn open(editor: &mut Editor, text: &str) {
        editor.feed_text(":e ssh://fixture/var/log/app.log<cr>");
        let ticket = ticket(editor);
        deliver(editor, ticket, text);
    }
}
