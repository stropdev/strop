//! Bounded source/view capture. Prefix text stays in the rope until worker work.
use super::words::MAX_WORD_BYTES;
use crate::editor::{Document, DocumentSource, Editor, InputOwner, Mode};
use ropey::Rope;
use std::path::PathBuf;
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::selection::Selection;
use strop_grammar::is_keyword;

pub(super) const MAX_COMPLETION_SELECTIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Invocation {
    Automatic,
    ManualCombined,
    ManualWords,
    ManualLanguage,
}
impl Invocation {
    pub fn manual(self) -> bool {
        self != Self::Automatic
    }
    pub fn words(self) -> bool {
        self != Self::ManualLanguage
    }
    pub fn language(self) -> bool {
        self != Self::ManualWords
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SourceIdentity {
    Scratch,
    Local {
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
    },
    Remote(strop_workspace::RemoteFile),
}
impl SourceIdentity {
    fn capture(document: &Document) -> Result<Self, ContextRefusal> {
        Ok(match &document.source {
            DocumentSource::Scratch => Self::Scratch,
            DocumentSource::File => {
                let path = document
                    .buf
                    .file_identity()
                    .or(document.buf.path.as_deref())
                    .ok_or(ContextRefusal::SourceUnavailable)?;
                if path.as_os_str().len() > MAX_IDENTITY_BYTES {
                    return Err(ContextRefusal::IdentityLimit);
                }
                Self::Local {
                    path: path.to_owned(),
                }
            }
            DocumentSource::Remote(source) => {
                bounded_identity(&source.file, MAX_IDENTITY_BYTES)?;
                Self::Remote(source.file.clone())
            }
            _ => return Err(ContextRefusal::UnsupportedSurface),
        })
    }
    fn matches(&self, document: &Document) -> bool {
        match (self, &document.source) {
            (Self::Scratch, DocumentSource::Scratch) => true,
            (Self::Local { path }, DocumentSource::File) => {
                document
                    .buf
                    .file_identity()
                    .or(document.buf.path.as_deref())
                    == Some(path.as_path())
            }
            (Self::Remote(file), DocumentSource::Remote(source)) => *file == source.file,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct SourceBinding {
    pub document: DocumentId,
    identity: SourceIdentity,
}
impl SourceBinding {
    pub fn matches(&self, editor: &Editor) -> bool {
        editor.docs.get(self.document).is_some_and(|document| {
            writable_source(editor, self.document, document) && self.identity.matches(document)
        })
    }
}

fn writable_source(editor: &Editor, id: DocumentId, document: &Document) -> bool {
    !document.buf.readonly
        && match &document.source {
            DocumentSource::File | DocumentSource::Scratch => true,
            DocumentSource::Remote(source) => {
                source.window.is_complete()
                    && !editor.remote_following(id)
                    && source
                        .write
                        .as_ref()
                        .is_some_and(|permit| permit.owns(&source.file))
            }
            // Container snapshots have no write capability, even if a caller
            // accidentally changes the buffer's presentation readonly flag.
            _ => false,
        }
}

/// A language-service incarnation includes placement, not only its process ID.
/// Shared across keystrokes; a prepared operation must still own this binding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct LanguageContext {
    server: strop_lsp::ServerId,
    #[serde(with = "strop_core::path_serde")]
    path: PathBuf,
    #[serde(with = "strop_core::path_serde")]
    root: PathBuf,
    language: String,
    target: strop_workspace::Filesystem,
}
impl LanguageContext {
    pub fn capture(
        binding: &crate::editor::lsp::state::Binding,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, ContextRefusal> {
        if let Some(previous) = previous.filter(|previous| previous.matches(binding)) {
            return Ok(previous.clone());
        }
        if binding.path.as_os_str().len() > MAX_IDENTITY_BYTES
            || binding.root.as_os_str().len() > MAX_IDENTITY_BYTES
            || binding.language.len() > 128
        {
            return Err(ContextRefusal::IdentityLimit);
        }
        bounded_identity(&binding.target, MAX_IDENTITY_BYTES)?;
        Ok(Arc::new(Self {
            server: binding.server,
            path: binding.path.clone(),
            root: binding.root.clone(),
            language: binding.language.clone(),
            target: binding.target.clone(),
        }))
    }

    fn matches(&self, binding: &crate::editor::lsp::state::Binding) -> bool {
        self.server == binding.server
            && self.path == binding.path
            && self.root == binding.root
            && self.language == binding.language
            && self.target == binding.target
    }

    pub fn owns(
        &self,
        editor: &Editor,
        context: &CompletionContext,
        stamp: strop_lsp::RequestStamp,
    ) -> bool {
        context.primary().is_some_and(|primary| {
            primary.source == stamp.document
                && editor
                    .lsp_state
                    .bindings
                    .get(&primary.source)
                    .is_some_and(|binding| {
                        binding.server == stamp.server
                            && binding.revision == primary.revision
                            && self.matches(binding)
                    })
        })
    }
}

const MAX_IDENTITY_BYTES: usize = 8192;
fn bounded_identity(value: &impl serde::Serialize, remaining: usize) -> Result<(), ContextRefusal> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or(std::io::ErrorKind::InvalidInput)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(remaining), value).map_err(|_| ContextRefusal::IdentityLimit)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionScope {
    pub view: DocumentId,
    pub pane: usize,
    pub focus_epoch: u64,
    #[serde(with = "strop_core::path_serde")]
    cwd: PathBuf,
    sources: Vec<SourceBinding>,
}
impl CompletionScope {
    fn matches(&self, editor: &Editor) -> bool {
        !editor.panes.is_empty()
            && self.view == editor.current()
            && self.pane == editor.active_pane
            && self.focus_epoch == editor.focus_epoch
            && self.cwd == editor.cwd
            && self.sources.iter().all(|source| source.matches(editor))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionTarget {
    pub source: DocumentId,
    pub revision: BufferRevision,
    pub selection: Selection,
    pub start: usize,
    pub caret: usize,
    pub end: usize,
    pub view_start: usize,
    pub view_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionContext {
    pub scope: Arc<CompletionScope>,
    pub view_revision: BufferRevision,
    pub view_bytes: usize,
    pub settings_generation: u64,
    pub invocation: Invocation,
    /// Primary first, then the exact selection shape. Equivalent stacked edits
    /// are deduplicated by acceptance, never by relabelling source ownership.
    pub targets: Vec<CompletionTarget>,
}

impl CompletionContext {
    pub fn primary(&self) -> Option<&CompletionTarget> {
        self.targets.first()
    }
    pub fn primary_binding(&self) -> Option<&SourceBinding> {
        let source = self.primary()?.source;
        self.scope
            .sources
            .iter()
            .find(|binding| binding.document == source)
    }

    pub fn still_owns(&self, editor: &Editor, settings_generation: u64) -> bool {
        if self.targets.is_empty()
            || self.targets.len() > MAX_COMPLETION_SELECTIONS
            || self.scope.sources.is_empty()
            || self.scope.sources.len() > MAX_COMPLETION_SELECTIONS
            || editor.finishing
            || editor.panes.is_empty()
            || !editor.config.completion.enabled
            || (!self.invocation.manual() && !editor.config.completion.auto_popup)
            || editor.mode != Mode::Insert
            || editor.input_owner() != InputOwner::Document
            || settings_generation != self.settings_generation
            || !self.scope.matches(editor)
            || editor.buf().revision() != self.view_revision
            || editor.view().sels.count() != self.targets.len()
        {
            return false;
        }
        let selections = std::iter::once(editor.view().sels.primary())
            .chain(editor.view().sels.extra_heads().iter().copied());
        self.targets
            .iter()
            .zip(selections)
            .all(|(target, selection)| {
                target.selection == selection
                    && editor
                        .docs
                        .get(target.source)
                        .is_some_and(|document| document.buf.revision() == target.revision)
                    && editor.source_edit_position(self.scope.view, selection.head)
                        == Some((target.source, target.caret))
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum ContextRefusal {
    #[error("code completion is disabled")]
    Disabled,
    #[error("automatic completion is disabled; manual completion remains available")]
    AutomaticDisabled,
    #[error("completion belongs to Insert mode in a source buffer")]
    NotSourceInsert,
    #[error("completion source is no longer available")]
    SourceUnavailable,
    #[error("completion does not edit generated headers, gaps or input fields")]
    UnsupportedSurface,
    #[error("completion requires a writable complete source buffer")]
    ReadOnly,
    #[error("completion selection exceeds the bounded 64-caret operation")]
    TooManySelections,
    #[error("completion requires compatible prefixes and suffixes at every caret")]
    MixedPrefixes,
    #[error("completion keyword exceeds the 256-byte operation bound")]
    KeywordTooLong,
    #[error("completion position is not a source character boundary")]
    InvalidPosition,
    #[error("completion source identity exceeds its bounded metadata representation")]
    IdentityLimit,
}

pub(super) fn capture(
    editor: &Editor,
    invocation: Invocation,
    settings_generation: u64,
    previous: Option<&Arc<CompletionScope>>,
) -> Result<CompletionContext, ContextRefusal> {
    if !editor.config.completion.enabled {
        return Err(ContextRefusal::Disabled);
    }
    if !invocation.manual() && !editor.config.completion.auto_popup {
        return Err(ContextRefusal::AutomaticDisabled);
    }
    if editor.cwd.as_os_str().len() > MAX_IDENTITY_BYTES {
        return Err(ContextRefusal::IdentityLimit);
    }
    if editor.finishing
        || editor.panes.is_empty()
        || editor.mode != Mode::Insert
        || editor.input_owner() != InputOwner::Document
    {
        return Err(ContextRefusal::NotSourceInsert);
    }
    let count = editor.view().sels.count();
    if count > MAX_COMPLETION_SELECTIONS {
        return Err(ContextRefusal::TooManySelections);
    }
    let mut targets: Vec<CompletionTarget> = Vec::with_capacity(count);
    for selection in std::iter::once(editor.view().sels.primary())
        .chain(editor.view().sels.extra_heads().iter().copied())
    {
        let (source, caret) = editor
            .source_edit_position(editor.current(), selection.head)
            .ok_or(ContextRefusal::UnsupportedSurface)?;
        let document = editor
            .docs
            .get(source)
            .ok_or(ContextRefusal::SourceUnavailable)?;
        if !matches!(
            document.source,
            DocumentSource::File
                | DocumentSource::Scratch
                | DocumentSource::Remote(_)
                | DocumentSource::Container { .. }
        ) {
            return Err(ContextRefusal::UnsupportedSurface);
        }
        if !writable_source(editor, source, document) {
            return Err(ContextRefusal::ReadOnly);
        }
        let (start, end) = keyword_span(document.buf.text(), caret)?;
        let view_start = selection
            .head
            .checked_sub(caret - start)
            .ok_or(ContextRefusal::UnsupportedSurface)?;
        let view_end = selection
            .head
            .checked_add(end - caret)
            .ok_or(ContextRefusal::InvalidPosition)?;
        // Both ends must be in the same authoritative excerpt; a completion
        // prefix must not reach backwards through generated collection chrome.
        if editor.source_edit_position(editor.current(), view_start) != Some((source, start))
            || editor.source_edit_position(editor.current(), view_end) != Some((source, end))
        {
            return Err(ContextRefusal::UnsupportedSurface);
        }
        if let Some(primary) = targets.first() {
            let original = editor
                .docs
                .get(primary.source)
                .ok_or(ContextRefusal::SourceUnavailable)?;
            if original.buf.text().byte_slice(primary.start..primary.caret)
                != document.buf.text().byte_slice(start..caret)
                || original.buf.text().byte_slice(primary.caret..primary.end)
                    != document.buf.text().byte_slice(caret..end)
            {
                return Err(ContextRefusal::MixedPrefixes);
            }
        }
        targets.push(CompletionTarget {
            source,
            revision: document.buf.revision(),
            selection,
            start,
            caret,
            end,
            view_start,
            view_end,
        });
    }
    let source_count = targets
        .iter()
        .enumerate()
        .filter(|(index, target)| {
            !targets[..*index]
                .iter()
                .any(|earlier| earlier.source == target.source)
        })
        .count();
    let scope = if let Some(scope) = previous.filter(|scope| {
        scope.matches(editor)
            && scope.sources.len() == source_count
            && targets.iter().all(|target| {
                scope
                    .sources
                    .iter()
                    .any(|source| source.document == target.source)
            })
    }) {
        Arc::clone(scope)
    } else {
        let mut sources: Vec<SourceBinding> = Vec::with_capacity(source_count);
        for target in &targets {
            if sources
                .iter()
                .any(|source| source.document == target.source)
            {
                continue;
            }
            let document = editor
                .docs
                .get(target.source)
                .ok_or(ContextRefusal::SourceUnavailable)?;
            sources.push(SourceBinding {
                document: target.source,
                identity: SourceIdentity::capture(document)?,
            });
        }
        Arc::new(CompletionScope {
            view: editor.current(),
            pane: editor.active_pane,
            focus_epoch: editor.focus_epoch,
            cwd: editor.cwd.clone(),
            sources,
        })
    };
    Ok(CompletionContext {
        scope,
        view_revision: editor.buf().revision(),
        view_bytes: editor.buf().len_bytes(),
        settings_generation,
        invocation,
        targets,
    })
}

fn keyword_span(rope: &Rope, caret: usize) -> Result<(usize, usize), ContextRefusal> {
    let character = rope
        .try_byte_to_char(caret)
        .map_err(|_| ContextRefusal::InvalidPosition)?;
    if rope.char_to_byte(character) != caret {
        return Err(ContextRefusal::InvalidPosition);
    }
    let mut start = caret;
    let mut left = rope.chars_at(character);
    while let Some(character) = left.prev() {
        if !is_keyword(character) {
            break;
        }
        if caret - start + character.len_utf8() > MAX_WORD_BYTES {
            return Err(ContextRefusal::KeywordTooLong);
        }
        start -= character.len_utf8();
    }
    let mut end = caret;
    for character in rope.chars_at(character) {
        if !is_keyword(character) {
            break;
        }
        if end - start + character.len_utf8() > MAX_WORD_BYTES {
            return Err(ContextRefusal::KeywordTooLong);
        }
        end += character.len_utf8();
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests;
