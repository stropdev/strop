use super::*;
impl FsState {
    pub(crate) fn unconfirmed(&self) -> usize {
        self.history
            .iter()
            .map(|attempt| {
                attempt
                    .receipts
                    .iter()
                    .filter(|receipt| receipt.outcome.is_unconfirmed())
                    .count()
            })
            .sum()
    }
}
impl Editor {
    pub(crate) fn filesystem_shutdown_error(&self) -> Option<String> {
        let stores = self.io.store_attempts().count();
        if self.filesystem.unconfirmed() == 0 && stores == 0 {
            return None;
        }
        let mut text = String::from(
            "filesystem outcomes remain unconfirmed; in-memory receipts are not persisted:",
        );
        for attempt in &self.filesystem.history {
            for receipt in attempt
                .receipts
                .iter()
                .filter(|receipt| receipt.outcome.is_unconfirmed())
            {
                text.push_str(&format!(
                    "\n  operation {} step {}: {}",
                    attempt.ticket.request.get(),
                    receipt.step + 1,
                    receipt.operation.intent.kind.label()
                ));
                for location in receipt
                    .operation
                    .intent
                    .source
                    .iter()
                    .chain(receipt.operation.intent.destination.iter())
                {
                    text.push(' ');
                    text.push_str(&location.label());
                }
            }
        }
        // Unconfirmed document stores hold the same in-memory-only
        // evidence (0058 WK09): report them rather than dying silently.
        for (_document, attempt) in self.io.store_attempts() {
            text.push_str(&format!(
                "\n  document save: {} — {}",
                attempt.receipt.operation.intent.kind.label(),
                strop_workspace::ResourceLocation::local(attempt.write_target.clone()).label()
            ));
        }
        Some(text)
    }
}
