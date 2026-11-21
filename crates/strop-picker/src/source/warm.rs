//! One running preparation and one replaceable root per editor. Only the
//! worker can block in enumeration; demand never joins this thread.
use super::{cache, SelectionPolicy};
use crate::query::{FileSelectionPlan, SearchQuery};
use parking_lot::{Condvar, Mutex};
use std::path::PathBuf;
use std::sync::Arc;
use strop_core::worker::{CancelHandle, CancelToken};

struct Request {
    root: PathBuf,
    query: Arc<SearchQuery>,
    policy: SelectionPolicy,
    token: CancelToken,
}

struct State {
    next: Option<Request>,
    owner: Option<CancelHandle>,
    closed: bool,
}

pub(super) struct Lane {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl Lane {
    pub(super) fn new(cache: Arc<Mutex<cache::Caches>>) -> std::io::Result<Self> {
        let state = Arc::new((
            Mutex::new(State {
                next: None,
                owner: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker = Arc::clone(&state);
        std::thread::Builder::new()
            .name("inventory-warm".into())
            .spawn(move || loop {
                let (slot, wake) = &*worker;
                let request = {
                    let mut state = slot.lock();
                    loop {
                        if state.closed {
                            return;
                        }
                        if let Some(request) = state.next.take() {
                            break request;
                        }
                        wake.wait(&mut state);
                    }
                };
                let Request {
                    root,
                    query,
                    policy,
                    token,
                } = request;
                if token.is_cancelled() {
                    continue;
                }
                let plan = match FileSelectionPlan::compile(&query) {
                    Ok(plan) => plan,
                    Err(error) => {
                        debug_assert!(false, "invalid empty workspace query: {error:?}");
                        continue;
                    }
                };
                // Warming publishes no answers; on failure the foreground performs
                // its own checked request and reports its own outcome.
                drop(cache::inventory(
                    &cache,
                    cache::ScanRequest {
                        root: &root,
                        query: &query,
                        plan: &plan,
                        policy,
                        need_symbols: true,
                    },
                    1,
                    &token,
                    &|| {},
                ));
            })?;
        Ok(Self { state })
    }

    pub(super) fn schedule(&self, root: PathBuf, query: Arc<SearchQuery>, policy: SelectionPolicy) {
        let (token, owner) = CancelToken::standalone();
        let (slot, wake) = &*self.state;
        let mut state = slot.lock();
        if state.closed {
            return;
        }
        state.owner = Some(owner);
        state.next = Some(Request {
            root,
            query,
            policy,
            token,
        });
        wake.notify_one();
    }

    pub(super) fn cancel(&self) {
        let mut state = self.state.0.lock();
        state.next.take();
        state.owner.take();
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        let (slot, wake) = &*self.state;
        let mut state = slot.lock();
        state.closed = true;
        state.next.take();
        state.owner.take();
        wake.notify_one();
    }
}
