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
        if self.filesystem.unconfirmed() == 0 {
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
        Some(text)
    }
}
