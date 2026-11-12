//! Collections close what they opened (0101 §6). A file a collection build
//! loaded in the background carries a loan until the user takes it — it is
//! on screen after an event, or it was edited. When nothing holds a loaned
//! file any more it is released: closed without moving any cursor.

use std::collections::HashSet;

use strop_core::id::DocumentId;

use super::Editor;

/// Whether one document may be closed by a sweep (0101 §6.1): it is still
/// on loan, no pane shows it, no collection or collection build holds it,
/// and its text is the text it was loaded with. A dirty buffer has a moved
/// revision by construction, so it is never releasable.
pub(crate) fn releasable(
    loaned: bool,
    shown: bool,
    held: bool,
    revision_moved: bool,
    dirty: bool,
) -> bool {
    loaned && !shown && !held && !revision_moved && !dirty
}

/// `1 file the collection opened has unsaved changes and stays open`.
fn kept_dirty_message(count: usize) -> String {
    match count {
        1 => "1 file the collection opened has unsaved changes and stays open".into(),
        _ => format!("{count} files the collection opened have unsaved changes and stay open"),
    }
}

/// After every event: a loaned document a pane shows is taken, for good.
/// This one check is what "visited" means — `Enter` on a hit, `]q`, `:e`,
/// the finder, a split — whichever way the pane was rebound.
pub(crate) fn take_shown(editor: &mut Editor) {
    for index in 0..editor.panes.len() {
        let id = editor.panes[index].doc;
        if let Some(document) = editor.docs.get_mut(id) {
            document.loan = None;
        }
    }
}

/// Documents something still needs: any collection's excerpt sources,
/// and the files the build in flight is collecting.
fn loan_holders(editor: &Editor) -> HashSet<DocumentId> {
    let mut held: HashSet<DocumentId> = editor
        .collections
        .values()
        .flat_map(|collection| collection.excerpts.iter().map(|excerpt| excerpt.source))
        .collect();
    if let Some(build) = &editor.collection_build {
        let sources = editor.collection_sources();
        held.extend(
            build
                .hits
                .iter()
                .filter_map(|hit| sources.get(&hit.location).copied()),
        );
    }
    held
}

/// The sweep, run where a holder goes away (a collection closes, is
/// refreshed or invalidated, a build is abandoned): every loaned
/// document nothing holds is released. One that was edited stays and
/// its loan ends — its undo history is the only way back. Saying so
/// when some of those are unsaved.
pub(crate) fn release_unheld(editor: &mut Editor) {
    let held = loan_holders(editor);
    let shown: HashSet<DocumentId> = editor.panes.iter().map(|pane| pane.doc).collect();
    let mut release = Vec::new();
    let mut kept = Vec::new();
    let mut kept_dirty = 0;
    for (id, document) in editor.docs.iter() {
        let Some(loaned_at) = document.loan else {
            continue;
        };
        let moved = document.buf.revision() != loaned_at;
        let dirty = document.buf.is_dirty();
        debug_assert!(
            !dirty || moved,
            "a dirty buffer has moved past the revision it was loaned at"
        );
        if releasable(true, shown.contains(&id), held.contains(&id), moved, dirty) {
            release.push(id);
        } else if moved && !held.contains(&id) {
            kept.push(id);
            kept_dirty += usize::from(dirty);
        }
    }
    for id in kept {
        if let Some(document) = editor.docs.get_mut(id) {
            document.loan = None;
        }
    }
    for id in &release {
        debug_assert!(
            !editor.docs.get(*id).is_some_and(|d| d.buf.is_dirty()),
            "a dirty buffer is never released"
        );
        let released = editor.release_document(*id);
        debug_assert!(
            released.affected.is_empty(),
            "a released file held no excerpt"
        );
    }
    if !release.is_empty() {
        editor.lsp_retire_workspace_servers();
        strop_trace::record_with(
            strop_trace::EventKind::JobFinished,
            || serde_json::json!({"service":"collection","result":"released","documents":release.len()}),
        );
    }
    if kept_dirty > 0 {
        editor.message = kept_dirty_message(kept_dirty);
    }
}

#[cfg(test)]
mod tests {
    use super::releasable;

    #[test]
    fn only_an_untouched_unshown_unheld_loan_is_released() {
        for bits in 0u8..32 {
            let [loaned, shown, held, moved, dirty] =
                [0, 1, 2, 3, 4].map(|bit| bits & (1 << bit) != 0);
            let expected = loaned && !shown && !held && !moved && !dirty;
            assert_eq!(
                releasable(loaned, shown, held, moved, dirty),
                expected,
                "loaned {loaned} shown {shown} held {held} moved {moved} dirty {dirty}"
            );
        }
        // the one case that releases, and dirt alone always keeps
        assert!(releasable(true, false, false, false, false));
        assert!(!releasable(true, false, false, false, true));
    }
}
