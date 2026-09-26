//! Notification request handling and the advisory native watch pump.

use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::{sync::atomic::Ordering, thread, time::Duration};

#[cfg(not(target_os = "linux"))]
use strop_worker_protocol::Refusal;
use strop_worker_protocol::ResultOutcome;
#[cfg(target_os = "linux")]
use strop_worker_protocol::WorkerMessage;
use strop_workspace::ResourceLocation;

use super::SessionState;

/// Advisory hints never block admitted requests; polling also bounds the
/// latency of subscription changes without a self-pipe.
#[cfg(target_os = "linux")]
const NOTIFY_TICK: Duration = Duration::from_millis(25);

#[cfg(target_os = "linux")]
pub(super) fn subscribe(
    shared: &Arc<SessionState>,
    scope: &ResourceLocation,
    recursive: bool,
) -> ResultOutcome {
    match shared.notify.lock().subscribe(scope, recursive) {
        Ok(subscribed) => ResultOutcome::Subscribed {
            subscription: subscribed.subscription,
            coverage: subscribed.coverage,
        },
        Err(error) => ResultOutcome::Refused {
            refusal: error.refusal(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn subscribe(
    _shared: &Arc<SessionState>,
    _scope: &ResourceLocation,
    _recursive: bool,
) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::Capability {
            capability: strop_worker_protocol::Capability::Notify,
        },
    }
}

#[cfg(target_os = "linux")]
pub(super) fn unsubscribe(
    shared: &Arc<SessionState>,
    subscription: strop_worker_protocol::Subscription,
) -> ResultOutcome {
    match shared.notify.lock().unsubscribe(subscription) {
        Ok(()) => ResultOutcome::Done,
        Err(error) => ResultOutcome::Refused {
            refusal: error.refusal(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn unsubscribe(
    _shared: &Arc<SessionState>,
    _subscription: strop_worker_protocol::Subscription,
) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::Capability {
            capability: strop_worker_protocol::Capability::Notify,
        },
    }
}

#[cfg(target_os = "linux")]
pub(super) fn start_notify_pump(shared: &Arc<SessionState>) -> thread::JoinHandle<()> {
    let worker = Arc::clone(shared);
    let manager = Arc::clone(&shared.notify);
    thread::spawn(move || {
        while !worker.stop.load(Ordering::Acquire) {
            let drained = {
                let mut manager = manager.lock();
                match manager.poll(Some(NOTIFY_TICK)) {
                    Ok(_) => manager.drain(),
                    Err(error) => {
                        worker.note(format_args!("notify poll: {error}"));
                        Ok(Vec::new())
                    }
                }
            };
            match drained {
                Ok(events) => {
                    for event in events {
                        worker.send(WorkerMessage::Event { event });
                    }
                }
                Err(error) => worker.note(format_args!("notify drain: {error}")),
            }
        }
    })
}
