use super::*;

impl Editor {
    pub(super) fn start_filesystem_preparation(
        &mut self,
        key: FsKey,
        work: impl FnOnce(worker::CancelToken) -> Outcome<PreparedFilesystem> + Send + 'static,
    ) -> Result<(), String> {
        if self.filesystem.pending() || self.filesystem.pending.is_some() {
            return Err("finish the current filesystem request or review first".into());
        }
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket { request, key };
        self.filesystem.preparing = Some(ticket.clone());
        self.message = "preparing the changes; nothing applied yet".into();
        let launch = self.tape.request("filesystem.prepare", &ticket);
        match launch {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.handle_filesystem(FsEvent::Prepared(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return Ok(());
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-fs-prepare",
            move |outcome| {
                let _ = tx.send(IoEvent::Filesystem(Box::new(FsEvent::Prepared(Box::new(
                    Completion { ticket, outcome },
                )))));
            },
            work,
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }
}
