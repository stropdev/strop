//! Query-driven language injections. Every parser reads the same frozen rope
//! using absolute included ranges; filenames and shebangs are detection data,
//! never filesystem operations. Unknown languages retain the outer treatment.
use super::{
    languages::{self, LanguageId, LanguageSpec},
    spans::LayeredSpan,
    HighlightError, Highlighter, RopeText,
};
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use strop_core::id::BufferRevision;
use tree_sitter::{Node, Query, QueryCursor, Range, Tree};

pub(crate) struct InjectedHighlighter {
    language: LanguageId,
    ranges: Vec<Range>,
    highlighter: Highlighter,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LanguageSource {
    Shebang,
    Filename,
    Explicit,
}
impl InjectedHighlighter {
    pub(crate) fn apply_edits(
        &mut self,
        edits: &[strop_core::InputEdit],
        revision: BufferRevision,
    ) {
        self.highlighter.apply_edits(edits, revision);
    }
}
#[derive(Clone, Copy)]
enum IncludedChildren {
    None,
    Unnamed,
    All,
}
struct Injection {
    spec: LanguageSpec,
    ranges: Vec<Range>,
    combined: bool,
    source: LanguageSource,
}

impl Highlighter {
    pub(crate) fn injection_spans(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        first: usize,
        last: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<LayeredSpan>, HighlightError> {
        let Some(query) = self.injection_query.as_ref() else {
            return Ok(Vec::new());
        };
        let tree = self.tree.as_ref().ok_or(HighlightError::Parse)?;
        let injections = collect(rope, tree, query, first, last, cancelled)?;
        if injections.is_empty() {
            self.children.clear();
            return Ok(Vec::new());
        }
        const MAX_DEPTH: usize = 8;
        if self.injection_depth >= MAX_DEPTH {
            return Err(HighlightError::InjectionDepth);
        }
        let mut previous = std::mem::take(&mut self.children);
        let mut output = Vec::new();
        for injection in injections {
            if cancelled() {
                return Err(HighlightError::Cancelled);
            }
            let index = previous
                .iter()
                .position(|child| {
                    child.language == injection.spec.id && child.ranges == injection.ranges
                })
                .or_else(|| {
                    previous
                        .iter()
                        .position(|child| child.language == injection.spec.id)
                });
            let mut child = if let Some(index) = index {
                previous.swap_remove(index)
            } else {
                let language = injection.spec.id;
                let highlighter =
                    Highlighter::from_spec(injection.spec).ok_or(HighlightError::Parse)?;
                InjectedHighlighter {
                    language,
                    ranges: Vec::new(),
                    highlighter,
                }
            };
            if child.ranges != injection.ranges {
                child
                    .highlighter
                    .parser
                    .set_included_ranges(&injection.ranges)
                    .map_err(|_| HighlightError::Parse)?;
                child.highlighter.invalidate();
                child.ranges = injection.ranges;
            }
            child.highlighter.injection_depth = self.injection_depth + 1;
            for span in child
                .highlighter
                .highlight_cancellable(rope, revision, first, last, cancelled)?
            {
                for range in &child.ranges {
                    let start = span.start.max(range.start_byte);
                    let end = span.end.min(range.end_byte);
                    if start < end {
                        output.push(LayeredSpan {
                            span: super::Span { start, end, ..span },
                            injected: true,
                        });
                    }
                }
            }
            self.children.push(child);
        }
        Ok(output)
    }
}

fn collect(
    rope: &Rope,
    tree: &Tree,
    query: &Query,
    first: usize,
    last: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Injection>, HighlightError> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(first..last);
    let mut progress = |_: &tree_sitter::QueryCursorState| cancelled();
    let mut matches = cursor.matches_with_options(
        query,
        tree.root_node(),
        RopeText { rope },
        tree_sitter::QueryCursorOptions::new().progress_callback(&mut progress),
    );
    let names = query.capture_names();
    let mut found: Vec<Injection> = Vec::new();
    while let Some(matched) = matches.next() {
        if cancelled() {
            return Err(HighlightError::Cancelled);
        }
        let mut language = None;
        let mut filename = None;
        let mut shebang = None;
        let mut contents = Vec::new();
        let mut children = IncludedChildren::None;
        let mut combined = false;
        for property in query.property_settings(matched.pattern_index) {
            match property.key.as_ref() {
                "injection.language" => language = property.value.as_deref().map(str::to_owned),
                "injection.include-children" => children = IncludedChildren::All,
                "injection.include-unnamed-children"
                    if !matches!(children, IncludedChildren::All) =>
                {
                    children = IncludedChildren::Unnamed
                }
                "injection.combined" => combined = true,
                _ => {}
            }
        }
        for capture in matched.captures {
            match names[capture.index as usize] {
                "injection.language" => language = Some(capture_text(rope, capture.node)),
                "injection.filename" => filename = Some(capture_text(rope, capture.node)),
                "injection.shebang" => shebang = Some(capture_text(rope, capture.node)),
                "injection.content" => contents.push(capture.node),
                _ => {}
            }
        }
        let selected = if let Some(language) = language {
            languages::for_name(&language).map(|spec| (spec, LanguageSource::Explicit))
        } else if let Some(filename) = filename {
            languages::detect(std::path::Path::new(&filename), None)
                .map(|spec| (spec, LanguageSource::Filename))
        } else if let Some(shebang) = shebang {
            languages::for_shebang(&shebang).map(|spec| (spec, LanguageSource::Shebang))
        } else {
            None
        };
        let Some((spec, source)) = selected else {
            continue;
        };
        let mut ranges = Vec::new();
        for node in contents {
            included_ranges(node, children, &mut ranges);
        }
        canonicalize(&mut ranges);
        if ranges.is_empty() {
            continue;
        }
        if let Some(index) = found.iter().position(|existing| existing.ranges == ranges) {
            if found[index].source <= source {
                found[index] = Injection {
                    spec,
                    ranges,
                    combined,
                    source,
                };
            }
        } else {
            found.push(Injection {
                spec,
                ranges,
                combined,
                source,
            });
        }
    }
    if cancelled() {
        return Err(HighlightError::Cancelled);
    }
    let mut result: Vec<Injection> = Vec::new();
    for injection in found {
        if let Some(existing) = result.iter_mut().find(|existing| {
            existing.combined && injection.combined && existing.spec.id == injection.spec.id
        }) {
            existing.ranges.extend(injection.ranges);
            canonicalize(&mut existing.ranges);
        } else {
            result.push(injection);
        }
    }
    Ok(result)
}

fn capture_text(rope: &Rope, node: Node<'_>) -> String {
    rope.byte_slice(node.start_byte()..node.end_byte())
        .chars()
        .take(256)
        .collect()
}
fn included_ranges(node: Node<'_>, policy: IncludedChildren, ranges: &mut Vec<Range>) {
    if matches!(policy, IncludedChildren::All) {
        ranges.push(node.range());
        return;
    }
    let mut start_byte = node.start_byte();
    let mut start_point = node.start_position();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(policy, IncludedChildren::Unnamed) && !child.is_named() {
            continue;
        }
        if start_byte < child.start_byte() {
            ranges.push(Range {
                start_byte,
                start_point,
                end_byte: child.start_byte(),
                end_point: child.start_position(),
            });
        }
        start_byte = child.end_byte();
        start_point = child.end_position();
    }
    if start_byte < node.end_byte() {
        ranges.push(Range {
            start_byte,
            start_point,
            end_byte: node.end_byte(),
            end_point: node.end_position(),
        });
    }
}
fn canonicalize(ranges: &mut Vec<Range>) {
    ranges.retain(|range| range.start_byte < range.end_byte);
    ranges.sort_by_key(|range| (range.start_byte, range.end_byte));
    let mut kept = 0;
    for index in 0..ranges.len() {
        let range = ranges[index];
        if kept > 0 && ranges[kept - 1].end_byte >= range.start_byte {
            if range.end_byte > ranges[kept - 1].end_byte {
                ranges[kept - 1].end_byte = range.end_byte;
                ranges[kept - 1].end_point = range.end_point;
            }
        } else {
            ranges[kept] = range;
            kept += 1;
        }
    }
    ranges.truncate(kept);
}
