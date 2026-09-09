//! Local-channel delivery for tests; live drivers use the shared AppEvent channel.
use super::picker::PickerEvent;
use super::{trace, Editor};
use std::sync::mpsc::TryRecvError;
use strop_core::worker::{FailureKind, Outcome};
use strop_picker::PickerMsg;

impl Editor {
    /// A named local repository for pure surface fixtures; no discovery or I/O.
    pub(crate) fn fixture_git_context(&mut self) {
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
