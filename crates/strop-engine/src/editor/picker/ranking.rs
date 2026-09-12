//! Correlated picker ranking: native CPU actors publish through the same AppEvent
//! and replay boundary as every other service. No scoring occurs on an input key.
use super::{Editor, PickerId};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use strop_core::worker::{FailureKind, Outcome, Ticket};
use strop_picker::{RankingEvent, RankingWorker};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Key {
    pub picker: PickerId,
    pub query: String,
    pub items: usize,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub picker: PickerId,
    pub update: RankingEvent<Key>,
}
pub(crate) struct State {
    pub tx: Sender<Event>,
    pub rx: Option<Receiver<Event>>,
    pub retiring: HashSet<PickerId>,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            retiring: HashSet::new(),
        }
    }
}
impl Editor {
    pub(super) fn start_picker_ranking(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let id = glue.id;
        glue.rank_alive = true;
        match self.tape.request("picker.ranker", &id) {
            Ok(false) => {
                self.request_picker_ranking();
                return;
            }
            Err(error) => {
                glue.rank_alive = false;
                glue.picker.error = Some(error.to_string());
                return;
            }
            Ok(true) => {}
        }
        let tx = self.picker_ranking.tx.clone();
        match RankingWorker::start(move |update| tx.send(Event { picker: id, update }).is_ok()) {
            Ok(worker) => glue.rank_worker = Some(worker),
            Err(error) => {
                glue.rank_alive = false;
                glue.picker.error = Some(format!("picker ranking unavailable: {error}"));
                return;
            }
        }
        self.request_picker_ranking();
    }

    pub(crate) fn request_picker_ranking(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        if !glue.rank_alive {
            return;
        }
        // staleness compares against the EFFECTIVE rank query, like the
        // install check below (0051: qualifiers never rank — `language:rust`
        // edits must not clear results while the needle is unchanged)
        let effective = glue
            .picker
            .rank_query
            .clone()
            .unwrap_or_else(|| glue.picker.input.text.clone());
        if glue.ranked_query.as_deref() != Some(effective.as_str()) {
            glue.picker.clear_results();
        }
        if glue.rank_pending.is_some() {
            if !glue.rank_dirty {
                if let Some(worker) = &glue.rank_worker {
                    worker.cancel_pending();
                }
            }
            glue.rank_dirty = true;
            return;
        }
        glue.rank_dirty = false;
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                glue.picker.error = Some(error.message);
                return;
            }
        };
        let filter = glue.picker.filter_request();
        let ticket = Ticket {
            request,
            key: Key {
                picker: glue.id,
                query: filter.query.clone(),
                items: filter.catalog.len(),
            },
        };
        glue.rank_pending = Some(ticket.clone());
        match self.tape.request("picker.rank", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_ranking(Event {
                    picker: ticket.key.picker,
                    update: RankingEvent::Completed(strop_core::worker::Completion {
                        ticket,
                        outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                    }),
                });
                return;
            }
        }
        let result = glue
            .rank_worker
            .as_ref()
            .ok_or_else(|| std::io::Error::other("picker ranking worker is absent"))
            .and_then(|worker| worker.submit(ticket.clone(), filter));
        if let Err(error) = result {
            self.handle_picker_ranking(Event {
                picker: ticket.key.picker,
                update: RankingEvent::Completed(strop_core::worker::Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Disconnected, error.to_string()),
                }),
            });
        }
    }

    pub(crate) fn handle_picker_ranking(&mut self, event: Event) {
        if matches!(&event.update, RankingEvent::Stopped) {
            self.picker_ranking.retiring.remove(&event.picker);
        }
        let Some(glue) = self.picker.as_mut().filter(|glue| glue.id == event.picker) else {
            return;
        };
        match event.update {
            RankingEvent::Completed(completion) => {
                if glue.rank_pending.as_ref() != Some(&completion.ticket) {
                    return;
                }
                glue.rank_pending = None;
                if std::mem::take(&mut glue.rank_dirty) {
                    self.request_picker_ranking();
                    return;
                }
                match completion.outcome {
                    Outcome::Success(ranking) => {
                        // staleness compares against the EFFECTIVE rank
                        // query (0051: qualifiers never rank — the needle
                        // may differ from the raw input)
                        let effective = glue
                            .picker
                            .rank_query
                            .clone()
                            .unwrap_or_else(|| glue.picker.input.text.clone());
                        if completion.ticket.key.query != effective
                            || !glue.picker.install_ranking(ranking)
                        {
                            return;
                        }
                        glue.ranked_query = Some(completion.ticket.key.query);
                    }
                    Outcome::Failed { failure, .. } => {
                        if failure.kind == strop_core::worker::FailureKind::Protocol {
                            if let Some(query) = &glue.query {
                                glue.query_highlights
                                    .push(strop_picker::query::HighlightSpan {
                                        range: query.content_range(),
                                        role: strop_picker::query::Role::Error,
                                    });
                            }
                        }
                        glue.picker.error = Some(failure.message);
                        glue.accept_when_ranked = false;
                    }
                    Outcome::Cancelled(_) => glue.accept_when_ranked = false,
                }
            }
            RankingEvent::Stopped => {
                glue.rank_alive = false;
                if glue.rank_pending.take().is_some() {
                    glue.picker.error = Some("picker ranking worker stopped".into());
                    glue.revoke(strop_core::worker::CancelReason::OwnerClosed);
                    glue.picker.streaming = false;
                }
            }
        }
        self.finish_search_refresh();
        self.finish_pending_picker_accept();
    }
}
