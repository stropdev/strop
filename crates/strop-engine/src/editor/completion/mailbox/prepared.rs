//! Acceptance owns a single consumable plan, not an Arc that would force the
//! mutation gateway to copy every replacement String on the interactive path.
use super::*;
use crate::editor::completion::acceptance::{AcceptanceRefusal, CompletionAcceptancePlan};
use crate::editor::completion::model::{CandidateKey, CompletionPayload, CompletionQuery};

pub(super) const PLAN_BYTES: usize = 288 * 1024;
pub(super) struct Ready {
    pub query: WorkerId,
    pub scope: DeliveryScope,
    pub plan: CompletionAcceptancePlan,
}

impl CompletionMailbox {
    pub fn prepared(
        &self,
        query: Arc<CompletionQuery>,
        scope: DeliveryScope,
        candidate: CandidateKey,
        plan: Result<CompletionAcceptancePlan, AcceptanceRefusal>,
    ) -> bool {
        let prepared = match plan {
            Err(refusal) => Err(refusal),
            Ok(plan) => {
                let ready = Ready {
                    query: query.request,
                    scope,
                    plan,
                };
                loop {
                    self.retire_native();
                    let mut state = self.state.lock();
                    if !self.owns(query.request, scope) {
                        return false;
                    }
                    if let Some(previous) = state.prepared.take() {
                        state.bytes -= PLAN_BYTES;
                        drop(state);
                        drop(previous);
                        continue;
                    }
                    if state.bytes + PLAN_BYTES <= MAX_RETAINED_BYTES {
                        state.prepared = Some(ready);
                        state.bytes += PLAN_BYTES;
                        state.high_water_bytes = state.high_water_bytes.max(state.bytes);
                        break;
                    }
                    if state
                        .retired
                        .iter()
                        .any(|entry| Arc::strong_count(&entry.delivery) == 1)
                    {
                        drop(state);
                        continue;
                    }
                    self.available.wait(&mut state);
                }
                Ok(())
            }
        };
        self.publish(CompletionDelivery {
            query,
            scope,
            payload: CompletionPayload::Acceptance {
                candidate,
                prepared,
            },
        })
    }

    /// The caller first checks the live source/selection independently. The
    /// owning tape call records this transfer; replay receives the same owned
    /// plan without starting a native provider or copying its edit strings.
    pub fn take_prepared(
        &self,
        query: WorkerId,
        scope: DeliveryScope,
    ) -> Option<CompletionAcceptancePlan> {
        let mut state = self.state.lock();
        if !self.owns(query, scope)
            || state
                .prepared
                .as_ref()
                .is_none_or(|plan| plan.query != query || plan.scope != scope)
        {
            return None;
        }
        let ready = state.prepared.take()?;
        state.bytes -= PLAN_BYTES;
        self.available.notify_all();
        Some(ready.plan)
    }
}
