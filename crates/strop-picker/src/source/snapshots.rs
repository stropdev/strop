//! Dirty-source matching shared by local and SSH providers in the captured scope.
use super::{
    flow::{BatchError, StreamSender},
    query::RECORD_LIMIT,
};
use crate::query::{ContentPlan, Evidence, RowFacts};
use std::{path::PathBuf, sync::Arc};
use strop_core::worker::CancelToken;
use strop_workspace::ResourceLocation;
/// An immutable dirty-source snapshot. Native identity stays separate from display.
pub struct SourceSnapshot {
    pub path: std::path::PathBuf,
    pub text: ropey::Rope,
}
/// The evidence sources a snapshot pass may consult (0063 §4): the
/// project catalog for `repo:` atoms and the mutable symbol index the
/// dirty texts overlay into.
pub(super) struct Sources<'a> {
    pub catalog: Option<&'a crate::source::catalog::ProjectCatalog>,
    pub symbols: Option<&'a mut crate::source::symbols::SymbolIndex>,
}

pub(super) fn emit_snapshots(
    root: &ResourceLocation,
    content: &ContentPlan,
    snapshots: Vec<SourceSnapshot>,
    paths: &mut Vec<PathBuf>,
    tx: &StreamSender,
    token: &CancelToken,
    sources: Sources<'_>,
) -> Result<(), SnapshotError> {
    let Sources {
        catalog,
        mut symbols,
    } = sources;
    let mut extractors = super::symbols::Extractors::default();
    for snapshot in snapshots {
        let Ok(relative) = snapshot.path.strip_prefix(&root.path) else {
            continue;
        };
        let Some(index) = paths.iter().position(|path| path == relative) else {
            continue;
        };
        paths.swap_remove(index);
        let location = ResourceLocation {
            filesystem: root.filesystem.clone(),
            path: snapshot.path.clone(),
        };
        // Dirty text is authoritative for `kind:` evidence too: the
        // overlay replaces whatever the disk pass extracted (0063 §2).
        if let Some(index) = symbols.as_deref_mut() {
            index.overlay(relative, &snapshot.text, &mut extractors, &|| {
                token.is_cancelled()
            });
        }
        emit_rope(
            &snapshot.text,
            relative,
            &location,
            content,
            Evidence {
                catalog,
                symbols: symbols.as_deref(),
            },
            tx,
            token,
        )?;
    }
    Ok(())
}

pub(super) enum SnapshotError {
    Match(String),
    Delivery(BatchError),
}

impl SnapshotError {
    pub(super) fn finish(self, tx: &StreamSender) -> strop_core::worker::Outcome<()> {
        match self {
            Self::Delivery(error) => error.finish(tx),
            Self::Match(message) => strop_core::worker::Outcome::failed(
                strop_core::worker::FailureKind::Protocol,
                message,
            ),
        }
    }
}

impl From<SnapshotError> for String {
    fn from(error: SnapshotError) -> Self {
        match error {
            SnapshotError::Match(message) => message,
            SnapshotError::Delivery(error) => error.message().to_owned(),
        }
    }
}

/// The caller has just verified membership of this native path. The rope is
/// the sole source even when it has no matches; the bool only schedules hints.
pub(super) fn emit_rope(
    rope: &ropey::Rope,
    relative: &std::path::Path,
    location: &ResourceLocation,
    content: &ContentPlan,
    evidence: Evidence<'_>,
    tx: &StreamSender,
    token: &CancelToken,
) -> Result<bool, SnapshotError> {
    let label = strop_workspace::directory::display_path(relative);
    let spelling = relative.to_string_lossy();
    let mut hit = false;
    for (line, text) in rope.lines().enumerate() {
        if line % 256 == 0 {
            tx.heartbeat();
        }
        if token.is_cancelled() || !tx.inventory_current() {
            return Err(SnapshotError::Delivery(BatchError::Cancelled));
        }
        if text.len_bytes() > RECORD_LIMIT {
            return Err(SnapshotError::Match(
                "open source line exceeds the 1 MiB search bound".into(),
            ));
        }
        let line_text: std::borrow::Cow<'_, str> = text.into();
        let text = line_text.strip_suffix('\n').unwrap_or(&line_text);
        let row = RowFacts {
            path: Some(&spelling),
            native_path: Some(relative),
            line: Some(line + 1),
            text,
            ..RowFacts::default()
        };
        if !content.admits(&row, &evidence) {
            continue;
        }
        let text = Arc::from(text);
        let mut items = Vec::new();
        super::query::append_matches(
            &mut items,
            content,
            location,
            &label,
            line + 1,
            &text,
            Some("buffer"),
        )
        .map_err(SnapshotError::Match)?;
        hit |= !items.is_empty();
        tx.batch(items, token).map_err(SnapshotError::Delivery)?;
    }
    Ok(hit)
}
