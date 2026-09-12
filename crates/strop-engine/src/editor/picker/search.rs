//! One bounded retained Search investigation, independent of its visible card.
use super::*;
use strop_workspace::{Filesystem, ResourceLocation};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchScope {
    pub root: ResourceLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SearchStamp {
    pub session: WorkerId,
    pub dataset: u64,
    pub intent: u64,
}

pub(crate) struct SearchContext {
    pub scope: SearchScope,
    pub stamp: SearchStamp,
    pub origin: super::super::jumps::JumpRecord,
    pub refreshing: bool,
    pub refresh_requested: bool,
    pub policy: (bool, bool),
    restore: Option<RestoreView>,
}

impl SearchContext {
    pub(super) fn discard_view_restore(&mut self) {
        self.restore = None;
    }
    pub(super) fn begin_view_restore(&mut self) {
        if let Some(restore) = self.restore.as_mut() {
            restore.selected_item = None;
            restore.top_item = None;
        }
    }
}
struct RestoreView {
    selected: Option<HitWitness>,
    top: Option<HitWitness>,
    selected_item: Option<usize>,
    top_item: Option<usize>,
}
struct HitWitness {
    path: PathBuf,
    line: usize,
    column: usize,
    length: usize,
    text: std::sync::Arc<str>,
}
impl HitWitness {
    fn from_item(item: &Item) -> Option<Self> {
        let Payload::Grep {
            path,
            line,
            col,
            match_len,
            line_text,
        } = &item.payload
        else {
            return None;
        };
        Some(Self {
            path: path.clone(),
            line: *line,
            column: *col,
            length: *match_len,
            text: line_text.clone(),
        })
    }
    fn matches(&self, item: &Item) -> bool {
        matches!(&item.payload, Payload::Grep {path,line,col,match_len,line_text}
            if self.path == *path && self.line == *line && self.column == *col
                && self.length == *match_len && self.text == *line_text)
    }
}

impl Editor {
    pub fn search_scope(&self) -> Option<&SearchScope> {
        self.picker
            .as_ref()?
            .search
            .as_ref()
            .map(|context| &context.scope)
    }

    pub fn picker_path(&self, path: &std::path::Path) -> PathBuf {
        self.search_scope()
            .map_or(&self.cwd, |scope| &scope.root.path)
            .join(path)
    }

    pub(crate) fn search_stamp(&self, session: WorkerId) -> Option<SearchStamp> {
        self.picker
            .iter()
            .chain(self.retained_search.iter())
            .filter_map(|glue| glue.search.as_ref())
            .find(|context| context.stamp.session == session)
            .map(|context| context.stamp)
    }
    pub(super) fn open_search_hit(&mut self, payload: Payload) {
        let Payload::Grep {
            path,
            line,
            col,
            match_len,
            line_text,
        } = payload
        else {
            self.message = "Search result has no supported source witness".into();
            return;
        };
        let Some(scope) = self.search_scope() else {
            self.message = "Search has no captured workspace scope".into();
            return;
        };
        let path = scope.root.path.join(path);
        let hit = ReplacementHit {
            line,
            col,
            match_len,
            text: line_text,
        };
        self.push_jump();
        self.close_picker();
        self.request_open(path, super::super::io::OpenIntent::SearchHit(hit));
    }

    pub(super) fn new_search_context(
        &mut self,
        scope: SearchScope,
    ) -> Result<SearchContext, String> {
        let session = self.worker_ids.allocate().map_err(|error| error.message)?;
        Ok(SearchContext {
            scope,
            stamp: SearchStamp {
                session,
                dataset: 0,
                intent: 0,
            },
            origin: self.jump_record(),
            refreshing: false,
            refresh_requested: false,
            policy: (
                self.config.search_show_hidden,
                self.config.search_respect_ignore,
            ),
            restore: None,
        })
    }

    pub fn open_search(&mut self, replacement: bool) {
        let scope = self
            .picker
            .iter()
            .chain(self.retained_search.iter())
            .find_map(|glue| glue.search.as_ref().map(|context| context.scope.clone()))
            .unwrap_or_else(|| SearchScope {
                root: ResourceLocation::local(self.cwd.clone()),
            });
        self.open_search_in(scope, replacement);
    }

    pub fn open_search_in(&mut self, scope: SearchScope, replacement: bool) {
        if scope.root.filesystem != Filesystem::Local || !scope.root.path.is_absolute() {
            self.message = "Search currently requires an absolute local-workspace scope; no filesystem fallback".into();
            return;
        }
        self.cancel_open(CancelReason::Superseded);
        if let Some(glue) = self.picker.as_mut().filter(|glue| {
            glue.search
                .as_ref()
                .is_some_and(|context| context.scope == scope)
        }) {
            if replacement {
                glue.picker.replacement_visible = true;
                if !glue.picker.input.text.is_empty() {
                    glue.picker.field = strop_picker::Field::Replace;
                }
                glue.suggestions = None;
                glue.accept_when_ranked = false;
            }
            return;
        }
        self.close_picker();
        let mut glue = match self.retained_search.take() {
            Some(mut glue)
                if glue
                    .search
                    .as_ref()
                    .is_some_and(|context| context.scope == scope) =>
            {
                if let Some(context) = glue.search.as_mut() {
                    context.origin = self.jump_record();
                    context.refresh_requested = true;
                }
                glue
            }
            old => {
                if let Some(old) = old {
                    self.retire_picker_model(old.picker);
                }
                let context = match self.new_search_context(scope) {
                    Ok(context) => context,
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                };
                let mut glue =
                    PickerGlue::diagnostics(Picker::search(Vec::new(), false, replacement));
                glue.search = Some(context);
                glue
            }
        };
        if replacement {
            glue.picker.replacement_visible = true;
            glue.picker.field = if glue.picker.input.text.is_empty() {
                strop_picker::Field::Search
            } else {
                strop_picker::Field::Replace
            };
        }
        self.set_picker(glue);
        // Refresh has query ownership even when With retains keyboard focus.
        self.restart_search_query();
    }

    pub(super) fn retain_search(&mut self, mut glue: PickerGlue) {
        if let Some(context) = glue.search.as_mut() {
            if glue.picker.current().is_some() || context.restore.is_none() {
                context.restore = Some(RestoreView {
                    selected: glue.picker.current().and_then(HitWitness::from_item),
                    top: glue
                        .picker
                        .rows
                        .get(glue.picker.scroll_top)
                        .and_then(|row| glue.picker.items.get(row.item))
                        .and_then(HitWitness::from_item),
                    selected_item: None,
                    top_item: None,
                });
            }
        }
        glue.rank_pending = None;
        glue.rank_dirty = false;
        glue.rank_alive = false;
        glue.accept_when_ranked = false;
        glue.suggestions = None;
        if let Some(old) = self.retained_search.replace(glue) {
            self.retire_picker_model(old.picker);
        }
    }

    fn retire_picker_model(&mut self, picker: Picker) {
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let id = PickerId(request);
        self.picker_ranking.retiring.insert(id);
        match self.tape.request("picker.reclaim", &id) {
            Ok(false) => return,
            Err(error) => {
                self.picker_ranking.retiring.remove(&id);
                self.message = format!("search retirement failed: {error}");
                return;
            }
            Ok(true) => {}
        }
        let tx = self.picker_ranking.tx.clone();
        match strop_picker::RankingWorker::<ranking::Key>::start(move |update| {
            tx.send(ranking::Event { picker: id, update }).is_ok()
        }) {
            Ok(worker) => {
                if let Err(error) = worker.retire(picker) {
                    self.message = format!("search retirement failed: {error}");
                }
            }
            Err(error) => {
                self.picker_ranking.retiring.remove(&id);
                self.message = format!("search retirement failed: {error}");
            }
        }
    }

    pub(super) fn search_intent_changed(&mut self) {
        let session = if let Some(glue) = self.picker.as_mut() {
            glue.accept_when_ranked = false;
            let Some(context) = glue.search.as_mut() else {
                return;
            };
            let Some(next) = context.stamp.intent.checked_add(1) else {
                glue.picker.error = Some("search edit generation exhausted".into());
                return;
            };
            context.stamp.intent = next;
            context.stamp.session
        } else {
            return;
        };
        self.invalidate_search_review(session);
    }

    pub(super) fn toggle_search_replacement(&mut self) {
        if let Some(glue) = self
            .picker
            .as_mut()
            .filter(|glue| glue.picker.kind == Kind::Search)
        {
            glue.picker.toggle_replacement();
            glue.accept_when_ranked = false;
            glue.suggestions = None;
        }
    }

    pub(super) fn observe_search_items(&mut self, items: &[Item]) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let Some(restore) = glue
            .search
            .as_mut()
            .and_then(|context| context.restore.as_mut())
        else {
            return;
        };
        let base = glue.picker.items.len();
        for (offset, item) in items.iter().enumerate() {
            if restore
                .selected
                .as_ref()
                .is_some_and(|wanted| wanted.matches(item))
            {
                restore.selected_item = Some(base + offset);
            }
            if restore
                .top
                .as_ref()
                .is_some_and(|wanted| wanted.matches(item))
            {
                restore.top_item = Some(base + offset);
            }
        }
    }

    pub(super) fn finish_search_refresh(&mut self) {
        let Some(glue) = self.picker.as_mut().filter(|glue| {
            glue.picker.kind == Kind::Search
                && !glue.picker.streaming
                && glue.active.is_none()
                && glue.rank_pending.is_none()
                && glue.picker.error.is_none()
        }) else {
            return;
        };
        let Some(context) = glue.search.as_mut().filter(|context| context.refreshing) else {
            return;
        };
        context.refreshing = false;
        let lost = glue.picker.finish_workset_refresh();
        let mut lost_selection = false;
        if let Some(restore) = context.restore.take() {
            if let Some(index) = restore.selected_item {
                glue.picker.selected = index;
            } else {
                lost_selection = restore.selected.is_some();
            }
            if let Some(index) = restore.top_item {
                glue.picker.scroll_top = index;
            }
        }
        if lost > 0 || lost_selection {
            glue.picker.warning = Some(format!(
                "refresh: {lost} workset decision(s) no longer matched{}",
                if lost_selection {
                    "; selected hit changed"
                } else {
                    ""
                }
            ));
        }
    }

    pub(crate) fn resume_search_after_review(&mut self, stamp: SearchStamp) {
        let Some(glue) = self.retained_search.as_ref().filter(|glue| {
            glue.search
                .as_ref()
                .is_some_and(|context| context.stamp.session == stamp.session)
        }) else {
            self.message = "the original Search investigation is no longer retained".into();
            return;
        };
        let Some(context) = glue.search.as_ref() else {
            return;
        };
        let scope = context.scope.clone();
        let origin = context.origin.clone();
        if self.docs.get(origin.document).is_some() {
            self.jump_to(origin);
        }
        self.open_search_in(scope, false);
    }
}
