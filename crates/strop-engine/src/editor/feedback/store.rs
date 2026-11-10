use super::model::{FeedbackEvent, FeedbackId, FeedbackLifecycle, FeedbackOwner, FeedbackRecord};

/// Ordinary resolved history is bounded. Unresolved records are allowed to
/// exceed this bound rather than silently losing a save barrier or its only
/// recovery action; `:explain` explicitly reports that saturated condition.
pub(crate) const FEEDBACK_HISTORY_BOUND: usize = 64;

#[derive(Default)]
pub(crate) struct FeedbackStore {
    records: Vec<FeedbackRecord>,
    next_id: u64,
    /// `:messages`: every summary this store presented, and the echo row's
    /// ephemeral messages.
    pub(crate) log: super::messages::MessageLog,
}

impl FeedbackStore {
    pub(crate) fn records(&self) -> &[FeedbackRecord] {
        &self.records
    }

    pub(crate) fn presented(&self) -> Option<&FeedbackRecord> {
        self.records
            .iter()
            .filter(|record| record.lifecycle() == FeedbackLifecycle::Active)
            .max_by_key(|record| (record.severity(), record.id()))
    }

    pub(crate) fn get(&self, id: FeedbackId) -> Option<&FeedbackRecord> {
        self.records.iter().find(|record| record.id() == id)
    }

    pub(crate) fn record(&mut self, event: FeedbackEvent) -> Option<FeedbackId> {
        self.log.push(Some(event.severity), &event.summary);
        if let Some(existing) = self.records.iter_mut().find(|record| {
            record.unresolved() && record.owner() == &event.owner && record.kind == event.kind
        }) {
            let id = existing.id();
            existing.replace(
                event.severity,
                event.summary,
                event.detail,
                event.reference,
                event.actions,
            );
            return Some(id);
        }

        let next = self.next_id.checked_add(1)?;
        self.next_id = next;
        let id = FeedbackId::new(next);
        self.records.push(FeedbackRecord::new(id, event));
        self.prune_resolved();
        Some(id)
    }

    pub(crate) fn dismiss(&mut self, id: FeedbackId) -> bool {
        let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.id() == id && record.unresolved())
        else {
            return false;
        };
        record.dismiss();
        true
    }

    /// Settle only the recovered owner. A success for any other owner leaves
    /// this owner's failed or unconfirmed record untouched.
    pub(crate) fn resolve_owner(
        &mut self,
        owner: &FeedbackOwner,
        kind: super::model::FeedbackKind,
    ) {
        for record in self
            .records
            .iter_mut()
            .filter(|record| record.unresolved() && record.owner() == owner && record.kind == kind)
        {
            record.resolve();
        }
        self.prune_resolved();
    }

    pub(crate) fn resolve_remote(
        &mut self,
        endpoint: &strop_workspace::RemoteEndpoint,
        kind: super::model::FeedbackKind,
    ) {
        for record in self.records.iter_mut().filter(|record| {
            record.unresolved()
                && record.kind == kind
                && matches!(record.owner(), FeedbackOwner::Remote { endpoint: current, .. } if current == endpoint)
        }) {
            record.resolve();
        }
        self.prune_resolved();
    }

    pub(crate) fn resolve_terminal(
        &mut self,
        session: strop_terminal::model::SessionId,
        kind: super::model::FeedbackKind,
    ) {
        for record in self.records.iter_mut().filter(|record| {
            record.unresolved()
                && record.kind == kind
                && matches!(record.owner(), FeedbackOwner::Terminal { session: current, .. }
                    if *current == session)
        }) {
            record.resolve();
        }
        self.prune_resolved();
    }

    pub(crate) fn unresolved(&self) -> impl Iterator<Item = &FeedbackRecord> {
        self.records.iter().filter(|record| record.unresolved())
    }

    pub(crate) fn saturated(&self) -> bool {
        self.records.len() > FEEDBACK_HISTORY_BOUND
    }

    fn prune_resolved(&mut self) {
        while self.records.len() > FEEDBACK_HISTORY_BOUND {
            let Some(oldest_resolved) = self.records.iter().position(|record| !record.unresolved())
            else {
                break;
            };
            self.records.remove(oldest_resolved);
        }
    }
}
