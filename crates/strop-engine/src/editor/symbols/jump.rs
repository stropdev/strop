//! Workspace-symbol jump evidence: what a selected row carries to the
//! open, and the checks that its target still names the symbol.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use strop_core::id::DocumentId;
use strop_lsp::ServerId;

use super::store::{self, Producer};
use super::{matches_display, source};
use crate::editor::Editor;

/// Native evidence kept off the trace. The action carries its query stamp;
/// an old open cannot consume a new selection's witness.
pub(crate) struct WorkspaceJump {
    pub query: u64,
    pub epoch: Option<Arc<AtomicBool>>,
    pub disk: Option<Arc<strop_picker::DiskShard>>,
    pub root: strop_workspace::ResourceLocation,
    pub workspace: store::Workspace,
}

/// Check a literal symbol at the reported start (not a prefix of another
/// identifier). Positions passed here have already been checked in the
/// producing server's encoding against this target.
pub(crate) fn token_at(text: &ropey::Rope, offset: usize, name: &str) -> bool {
    let end = offset.saturating_add(name.len());
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !name.is_empty()
        && text
            .get_byte_slice(offset..end)
            .is_some_and(|slice| slice == name)
        && !(name.chars().next().is_some_and(word)
            && text
                .byte_to_char(offset)
                .checked_sub(1)
                .and_then(|i| text.get_char(i))
                .is_some_and(word))
        && !(name.chars().next_back().is_some_and(word)
            && text.get_char(text.byte_to_char(end)).is_some_and(word))
}

pub(crate) fn corroborates(
    editor: &mut Editor,
    document: DocumentId,
    server: ServerId,
    position: strop_lsp::ServerPosition,
    offset: usize,
    label: &str,
) -> bool {
    let producer = Producer::Server(server);
    if source(editor, document) != Some(producer) {
        return false;
    }
    let revision = editor.doc(document).buf.revision();
    editor
        .analysis
        .symbols
        .store
        .lookup(document, producer, revision)
        .is_some_and(|list| {
            matches_display(editor, document, &list)
                && list.rows.iter().any(|symbol| {
                    symbol.location.position == position
                        && label.ends_with(&symbol.name)
                        && token_at(editor.doc(document).buf.text(), offset, &symbol.name)
                })
        })
}
