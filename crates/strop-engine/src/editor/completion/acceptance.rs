//! Worker-side acceptance preparation. The live transaction gateway rechecks
//! the captured context and revision before publishing this whole operation.
use super::context::CompletionContext;
use ropey::Rope;
use strop_core::id::{BufferRevision, DocumentId, LineIndex};
use strop_core::{Range, Replacement};
use strop_lsp::completion::{CompletionItem, CompletionTextEdit, InsertTextFormat, InsertTextMode};
use strop_lsp::{PositionEncoding, ServerColumn, ServerPosition};

const MAX_ADDITIONAL_EDITS: usize = 64;
const MAX_ACCEPTANCE_EDITS: usize = 128;
const MAX_ACCEPTANCE_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum AcceptanceTarget {
    View {
        document: DocumentId,
        revision: BufferRevision,
    },
    Source {
        document: DocumentId,
        revision: BufferRevision,
    },
}
impl AcceptanceTarget {
    pub fn document(self) -> DocumentId {
        match self {
            Self::View { document, .. } | Self::Source { document, .. } => document,
        }
    }
    pub fn revision(self) -> BufferRevision {
        match self {
            Self::View { revision, .. } | Self::Source { revision, .. } => revision,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CompletionAcceptancePlan {
    pub target: AcceptanceTarget,
    pub edits: Vec<Replacement>,
    /// Primary first, in the target's coordinate domain after every represented
    /// edit (including imports). Selection is collapsed at these accepted ends.
    pub caret_bytes: Vec<usize>,
    pub recording: InsertRecordingUpdate,
    /// 0069 D05: snippet tabstops (index, per-target offsets in post-edit
    /// coordinates, primary first). `None` for plain-text acceptances.
    /// `#[serde(default)]`: tapes recorded before snippets decode clean.
    #[serde(default)]
    pub snippet: Option<Vec<SnippetStop>>,
}

/// One snippet tabstop occurrence (repeated indexes are linked stops).
/// `offsets` are (start, end) byte offsets, one per mirrored target,
/// primary first; a zero-length offset is a caret-only stop (e.g. `$0`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnippetStop {
    pub index: u32,
    pub offsets: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct InsertRecordingUpdate {
    pub remove_chars: usize,
    pub append: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize,
)]
pub(super) enum AcceptanceRefusal {
    #[error("completion source or selection is no longer available")]
    MissingSource,
    #[error("completion keyword is invalid or exceeds its bound")]
    Keyword,
    #[error("snippet completion is unsupported; no placeholder text was inserted")]
    Snippet,
    #[error("completion requires an unsupported whitespace adjustment")]
    WhitespaceMode,
    #[error("completion requires a server command that strop does not execute")]
    Command,
    #[error("completion must resolve its required edit data before acceptance")]
    ResolveRequired,
    #[error("semantic completion requires matching carets in one source document")]
    MixedSources,
    #[error("semantic completion does not apply to every selected prefix")]
    SelectionMismatch,
    #[error("completion contains an invalid source edit range")]
    InvalidRange,
    #[error("completion edits overlap or share an insertion position")]
    Overlap,
    #[error("completion operation exceeds its edit count or text-byte bound")]
    Limit,
}

struct Edit<'a> {
    range: Range,
    text: &'a str,
}

pub(super) fn word(
    context: &CompletionContext,
    text: &str,
    recorded_chars: usize,
) -> Result<CompletionAcceptancePlan, AcceptanceRefusal> {
    if context.targets.is_empty() {
        return Err(AcceptanceRefusal::MissingSource);
    }
    if text.is_empty()
        || text.len() > super::words::MAX_WORD_BYTES
        || !text.chars().all(strop_grammar::is_keyword)
    {
        return Err(AcceptanceRefusal::Keyword);
    }
    let mut edits: Vec<Edit<'_>> = Vec::with_capacity(context.targets.len());
    let mut primaries = Vec::with_capacity(context.targets.len());
    for target in &context.targets {
        let range = Range::charwise(target.view_start, target.view_end);
        primaries.push(range);
        if !edits.iter().any(|edit| edit.range == range) {
            edits.push(Edit { range, text });
        }
    }
    let primary = &context.targets[0];
    let prefix_chars = text
        .get(..primary.caret - primary.start)
        .ok_or(AcceptanceRefusal::Keyword)?
        .chars()
        .count();
    finish(
        AcceptanceTarget::View {
            document: context.scope.view,
            revision: context.view_revision,
        },
        context.view_bytes,
        edits,
        &primaries,
        prefix_chars,
        recorded_chars,
        Vec::new(),
    )
}

pub(super) fn unsupported_item(item: &CompletionItem) -> Option<AcceptanceRefusal> {
    let item = &item.protocol();
    let format = item
        .insert_text_format
        .unwrap_or(InsertTextFormat::PLAIN_TEXT);
    // Snippets are supported (0069 D05); an unparsable one refuses at
    // expansion time with the same named refusal, never a partial insert.
    if !matches!(
        format,
        InsertTextFormat::PLAIN_TEXT | InsertTextFormat::SNIPPET
    ) {
        Some(AcceptanceRefusal::Snippet)
    } else if item.insert_text_mode.unwrap_or(InsertTextMode::AS_IS) != InsertTextMode::AS_IS {
        Some(AcceptanceRefusal::WhitespaceMode)
    } else if item.command.is_some() {
        Some(AcceptanceRefusal::Command)
    } else {
        None
    }
}

/// `resolved` overlays only properties actually advertised for lazy resolution.
/// Original primary edits/data never disappear because a resolver omitted them.
pub(super) fn language(
    context: &CompletionContext,
    source: &Rope,
    encoding: PositionEncoding,
    item: &CompletionItem,
    resolved: Option<&CompletionItem>,
    resolve_required: bool,
    recorded_chars: usize,
) -> Result<CompletionAcceptancePlan, AcceptanceRefusal> {
    let original = &item.protocol();
    if let Some(refusal) = unsupported_item(item) {
        return Err(refusal);
    }
    if resolved.is_some_and(|item| item.protocol().command.is_some()) {
        return Err(AcceptanceRefusal::Command);
    }
    if resolve_required && resolved.is_none() {
        return Err(AcceptanceRefusal::ResolveRequired);
    }
    let primary = context.primary().ok_or(AcceptanceRefusal::MissingSource)?;
    if context
        .targets
        .iter()
        .any(|target| target.source != primary.source || target.revision != primary.revision)
    {
        return Err(AcceptanceRefusal::MixedSources);
    }
    let (range, text) = match &original.text_edit {
        Some(CompletionTextEdit::Edit(edit)) => {
            let range = primary_range(source, edit.range, encoding, primary.caret)?;
            (range, edit.new_text.as_str())
        }
        Some(CompletionTextEdit::InsertAndReplace(edit)) => {
            if edit.insert.start != edit.replace.start || edit.insert.end > edit.replace.end {
                return Err(AcceptanceRefusal::InvalidRange);
            }
            primary_range(source, edit.insert, encoding, primary.caret)?;
            let range = primary_range(source, edit.replace, encoding, primary.caret)?;
            (range, edit.new_text.as_str())
        }
        None => (
            Range::charwise(primary.start, primary.end),
            original
                .insert_text
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or(&original.label),
        ),
    };
    // 0069 D05: snippet items expand at plan time — the replacement text
    // is the expanded form and tabstops travel as typed offsets.
    let snippet_format = original
        .insert_text_format
        .unwrap_or(InsertTextFormat::PLAIN_TEXT)
        == InsertTextFormat::SNIPPET;
    let expanded;
    let mut snippet_stops: Vec<(u32, usize, usize)> = Vec::new();
    let text = if snippet_format {
        let parsed =
            strop_lsp::completion::snippet::parse(text).map_err(|_| AcceptanceRefusal::Snippet)?;
        let mut at = 0usize;
        for segment in &parsed.segments {
            use strop_lsp::completion::snippet::Segment;
            match segment {
                Segment::Text(run) => at += run.len(),
                Segment::Tabstop { index, default } => {
                    snippet_stops.push((*index, at, at + default.len()));
                    at += default.len();
                }
                Segment::Choice { index, options } => {
                    snippet_stops.push((*index, at, at + options[0].len()));
                    at += options[0].len();
                }
                Segment::Final => snippet_stops.push((0, at, at)),
            }
        }
        expanded = parsed.text;
        expanded.as_str()
    } else {
        text
    };
    if primary.caret - range.start.get() > strop_lsp::completion::MAX_FILTER_BYTES {
        return Err(AcceptanceRefusal::Limit);
    }
    let prefix_chars = source
        .byte_slice(range.start.get()..primary.caret)
        .chars()
        .count();
    let mut edits: Vec<Edit<'_>> = Vec::with_capacity(context.targets.len());
    let mut primaries = Vec::with_capacity(context.targets.len());
    for target in &context.targets {
        let mirrored = if context.targets.len() == 1 {
            range
        } else {
            if range.start.get() < primary.start || range.end.get() > primary.end {
                return Err(AcceptanceRefusal::SelectionMismatch);
            }
            Range::charwise(
                target.start + (range.start.get() - primary.start),
                target.start + (range.end.get() - primary.start),
            )
        };
        primaries.push(mirrored);
        if !edits.iter().any(|edit| edit.range == mirrored) {
            edits.push(Edit {
                range: mirrored,
                text,
            });
        }
    }
    let additional = resolved
        .and_then(|item| item.protocol().additional_text_edits.as_deref())
        .or(original.additional_text_edits.as_deref())
        .unwrap_or(&[]);
    if additional.len() > MAX_ADDITIONAL_EDITS {
        return Err(AcceptanceRefusal::Limit);
    }
    for edit in additional {
        edits.push(Edit {
            range: server_range(source, edit.range, encoding)?,
            text: &edit.new_text,
        });
    }
    finish(
        AcceptanceTarget::Source {
            document: primary.source,
            revision: primary.revision,
        },
        source.len_bytes(),
        edits,
        &primaries,
        prefix_chars,
        recorded_chars,
        snippet_stops,
    )
}

fn primary_range(
    source: &Rope,
    range: strop_lsp::completion::Range,
    encoding: PositionEncoding,
    caret: usize,
) -> Result<Range, AcceptanceRefusal> {
    if range.start.line != range.end.line {
        return Err(AcceptanceRefusal::InvalidRange);
    }
    let range = server_range(source, range, encoding)?;
    if range.start.get() > caret || range.end.get() < caret {
        return Err(AcceptanceRefusal::InvalidRange);
    }
    Ok(range)
}

fn server_range(
    source: &Rope,
    range: strop_lsp::completion::Range,
    encoding: PositionEncoding,
) -> Result<Range, AcceptanceRefusal> {
    let offset = |position: strop_lsp::completion::Position| {
        strop_lsp::checked_byte_offset(
            source,
            ServerPosition {
                line: LineIndex::new(position.line as usize),
                column: ServerColumn::new(position.character as usize),
            },
            encoding,
        )
        .map_err(|_| AcceptanceRefusal::InvalidRange)
    };
    let start = offset(range.start)?;
    let end = offset(range.end)?;
    if start > end {
        return Err(AcceptanceRefusal::InvalidRange);
    }
    Ok(Range::charwise(start, end))
}

fn finish(
    target: AcceptanceTarget,
    length: usize,
    mut edits: Vec<Edit<'_>>,
    primaries: &[Range],
    prefix_chars: usize,
    recorded_chars: usize,
    snippet_stops: Vec<(u32, usize, usize)>,
) -> Result<CompletionAcceptancePlan, AcceptanceRefusal> {
    if edits.len() > MAX_ACCEPTANCE_EDITS {
        return Err(AcceptanceRefusal::Limit);
    }
    let mut bytes = 0usize;
    for edit in &edits {
        bytes = bytes
            .checked_add(edit.text.len())
            .ok_or(AcceptanceRefusal::Limit)?;
        if bytes > MAX_ACCEPTANCE_BYTES {
            return Err(AcceptanceRefusal::Limit);
        }
    }
    edits.sort_unstable_by_key(|edit| (edit.range.start, edit.range.end));
    strop_core::editmap::check_batch(
        length,
        edits
            .iter()
            .map(|edit| (edit.range.start.get(), edit.range.end.get()))
            .collect(),
    )
    .map_err(|()| AcceptanceRefusal::Overlap)?;
    // Earlier edits shift everything behind them (imports sit ahead of
    // the primary); tabstops take the same ride as the caret.
    let shift_before = |edits: &[Edit<'_>], primary: &Range| -> isize {
        let mut shift = 0isize;
        for edit in edits {
            if edit.range.start >= primary.start {
                break;
            }
            shift +=
                edit.text.len() as isize - (edit.range.end.get() - edit.range.start.get()) as isize;
        }
        shift
    };
    let mut caret_bytes = Vec::with_capacity(primaries.len());
    for primary in primaries {
        let inserted = edits
            .iter()
            .find(|edit| edit.range == *primary)
            .ok_or(AcceptanceRefusal::InvalidRange)?;
        let mut caret = primary
            .start
            .get()
            .checked_add(inserted.text.len())
            .ok_or(AcceptanceRefusal::Limit)?;
        for edit in &edits {
            if edit.range.start >= primary.start {
                break;
            }
            caret = caret
                .checked_sub(edit.range.end.get() - edit.range.start.get())
                .and_then(|caret| caret.checked_add(edit.text.len()))
                .ok_or(AcceptanceRefusal::Limit)?;
        }
        caret_bytes.push(caret);
    }
    let snippet = if snippet_stops.is_empty() {
        None
    } else {
        let stops = snippet_stops
            .iter()
            .map(|(index, start, end)| SnippetStop {
                index: *index,
                offsets: primaries
                    .iter()
                    .map(|primary| {
                        let shift = shift_before(&edits, primary);
                        let base = primary.start.get() as isize + shift;
                        (
                            (base + *start as isize).max(0) as usize,
                            (base + *end as isize).max(0) as usize,
                        )
                    })
                    .collect(),
            })
            .collect();
        Some(stops)
    };
    let primary = primaries.first().ok_or(AcceptanceRefusal::MissingSource)?;
    let inserted = edits
        .iter()
        .find(|edit| edit.range == *primary)
        .ok_or(AcceptanceRefusal::InvalidRange)?;
    let remove_chars = prefix_chars.min(recorded_chars);
    let recording = InsertRecordingUpdate {
        remove_chars,
        append: inserted
            .text
            .chars()
            .skip(prefix_chars - remove_chars)
            .collect(),
    };
    let edits = edits
        .into_iter()
        .map(|edit| Replacement::new(edit.range, edit.text))
        .collect();
    Ok(CompletionAcceptancePlan {
        target,
        edits,
        caret_bytes,
        recording,
        snippet,
    })
}

#[cfg(test)]
mod tests;
