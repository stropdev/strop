//! Query-driven language injections. Every parser reads the same frozen rope
//! using absolute included ranges; filenames and shebangs are detection data,
//! never filesystem operations. Unknown languages retain the outer treatment.
use super::{
    compiled::Compiled,
    languages::{self, LanguageId},
    spans::LayeredSpan,
    HighlightError, Highlighter, RopeText,
};
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use strop_core::id::BufferRevision;
use tree_sitter::{Node, Query, QueryCursor, QueryMatch, Range, Tree};

mod documentation;

/// Nesting bound for injected layers. Past it the deeper layers keep
/// their host's treatment — what an unknown language gets — and the rest
/// of the window keeps its colour (0111 decision 2).
const MAX_DEPTH: usize = 8;

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
    language: LanguageId,
    ranges: Vec<Range>,
    combined: bool,
    source: LanguageSource,
}

impl Highlighter {
    pub(crate) fn injection_spans(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        window: (usize, usize),
        segments: &[(usize, usize)],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<LayeredSpan>, HighlightError> {
        let compiled = self.compiled;
        if self.injection_depth >= MAX_DEPTH
            || (compiled.windowed.is_none() && compiled.combined.is_none())
        {
            self.children.clear();
            return Ok(Vec::new());
        }
        let tree = self.tree.as_ref().ok_or(HighlightError::Parse)?;
        let injections = collect(
            rope,
            tree,
            compiled,
            &self.parser.included_ranges(),
            segments,
            cancelled,
        )?;
        let mut previous = std::mem::take(&mut self.children);
        // Exact matches first, for every injection, so one shifted block
        // cannot steal another block's tree; then, in document order, a
        // same-language child whose block moved or changed size.
        let mut assigned: Vec<Option<InjectedHighlighter>> = injections
            .iter()
            .map(|injection| {
                previous
                    .iter()
                    .position(|child| {
                        child.language == injection.language && child.ranges == injection.ranges
                    })
                    .map(|index| previous.remove(index))
            })
            .collect();
        for (injection, slot) in injections.iter().zip(assigned.iter_mut()) {
            if slot.is_none() {
                *slot = previous
                    .iter()
                    .position(|child| child.language == injection.language)
                    .map(|index| previous.remove(index));
            }
        }
        let mut output = Vec::new();
        for (injection, slot) in injections.into_iter().zip(assigned) {
            if cancelled() {
                return Err(HighlightError::Cancelled);
            }
            let mut child = match slot {
                Some(child) => child,
                None => InjectedHighlighter {
                    language: injection.language,
                    ranges: Vec::new(),
                    highlighter: Highlighter::for_language(injection.language)
                        .ok_or(HighlightError::Parse)?,
                },
            };
            if child.ranges != injection.ranges {
                child.highlighter.rebind(&injection.ranges)?;
                child.ranges = injection.ranges;
            }
            child.highlighter.injection_depth = self.injection_depth + 1;
            let spans = child
                .highlighter
                .spans_in(rope, revision, window, segments, cancelled)?;
            for span in spans {
                let first_range = child
                    .ranges
                    .partition_point(|range| range.end_byte <= span.start);
                for range in child.ranges[first_range..]
                    .iter()
                    .take_while(|range| range.start_byte < span.end)
                {
                    let start = span.start.max(range.start_byte);
                    let end = span.end.min(range.end_byte);
                    if start < end {
                        output.push(LayeredSpan {
                            span: super::Span {
                                start,
                                end,
                                ..*span
                            },
                            injected: true,
                        });
                    }
                }
            }
            self.children.push(child);
        }
        Ok(output)
    }

    /// New included ranges for an injected layer (0111 decision 5).
    ///
    /// The journal edits a kept tree's included ranges along with its
    /// nodes, so when a block only moved or grew with the edit — typing
    /// inside it, or anywhere before it — the edited tree already has
    /// exactly the new ranges and the reparse is an ordinary incremental
    /// one. Any other change (a fence closed or opened, a layer rebound
    /// to a different block) parses from nothing: tree-sitter's own diff
    /// of differing included ranges disagreed with a fresh parse under
    /// the seeded property in `tests/properties.rs`.
    fn rebind(&mut self, ranges: &[Range]) -> Result<(), HighlightError> {
        self.parser
            .set_included_ranges(ranges)
            .map_err(|_| HighlightError::Parse)?;
        if self
            .tree
            .as_ref()
            .is_some_and(|tree| tree.included_ranges() != ranges)
        {
            self.tree = None;
            self.children.clear();
        }
        self.source_hash = None;
        self.span_window = None;
        Ok(())
    }

    /// The first direct injected layer of `language` (test inspection).
    #[cfg(test)]
    pub(crate) fn injected_tree(&self, language: LanguageId) -> Option<&Tree> {
        self.children
            .iter()
            .find(|child| child.language == language)
            .and_then(|child| child.highlighter.tree.as_ref())
    }

    /// Whether any layer's tree holds an error or missing node (test
    /// inspection): tree-sitter's error recovery may legitimately settle
    /// differently after an incremental reparse than from scratch.
    #[cfg(test)]
    pub(crate) fn any_layer_has_error(&self) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|tree| tree.root_node().has_error())
            || self
                .children
                .iter()
                .any(|child| child.highlighter.any_layer_has_error())
    }
}

/// The injections a window needs: windowed patterns matched inside the
/// segments, combined patterns over the whole tree.
fn collect(
    rope: &Rope,
    tree: &Tree,
    compiled: &Compiled,
    parent_ranges: &[Range],
    segments: &[(usize, usize)],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Injection>, HighlightError> {
    let mut collector = Collector::default();
    let mut cursor = QueryCursor::new();
    let mut progress = |_: &tree_sitter::QueryCursorState| cancelled();
    let mut scan = |query: &Query,
                    range: std::ops::Range<usize>,
                    collector: &mut Collector|
     -> Result<(), HighlightError> {
        cursor.set_byte_range(range);
        let mut matches = cursor.matches_with_options(
            query,
            tree.root_node(),
            RopeText { rope },
            tree_sitter::QueryCursorOptions::new().progress_callback(&mut progress),
        );
        while let Some(matched) = matches.next() {
            if cancelled() {
                return Err(HighlightError::Cancelled);
            }
            collector.absorb(query, matched, rope, parent_ranges, cancelled)?;
        }
        Ok(())
    };
    if let Some(query) = &compiled.windowed {
        for &(start, end) in segments {
            scan(query, start..end, &mut collector)?;
        }
    }
    if let Some(query) = &compiled.combined {
        scan(query, 0..usize::MAX, &mut collector)?;
    }
    if cancelled() {
        return Err(HighlightError::Cancelled);
    }
    Ok(collector.finish())
}

#[derive(Default)]
struct Collector {
    found: Vec<Injection>,
    unknown: Vec<(Vec<Range>, LanguageSource)>,
    documentation_groups: Vec<documentation::Group>,
}

impl Collector {
    fn absorb(
        &mut self,
        query: &Query,
        matched: &QueryMatch<'_, '_>,
        rope: &Rope,
        parent_ranges: &[Range],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), HighlightError> {
        let names = query.capture_names();
        let mut language = None;
        let mut filename = None;
        let mut shebang = None;
        let mut contents = Vec::new();
        let mut children = IncludedChildren::None;
        let mut combined = false;
        let mut documentation = None;
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
                "injection.documentation" => {
                    documentation = documentation::Policy::from_property(property.value.as_deref());
                }
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
        let (detected, source) = if let Some(language) = language {
            (
                languages::for_name(&language).map(|spec| spec.id),
                LanguageSource::Explicit,
            )
        } else if let Some(filename) = filename {
            (
                languages::detect_id(std::path::Path::new(&filename), None),
                LanguageSource::Filename,
            )
        } else if let Some(shebang) = shebang {
            (
                languages::for_shebang(&shebang).map(|spec| spec.id),
                LanguageSource::Shebang,
            )
        } else {
            return Ok(());
        };
        let mut ranges = Vec::new();
        for node in contents {
            if let Some(policy) = documentation {
                ranges.extend(policy.ranges(
                    rope,
                    node,
                    matched.pattern_index,
                    &mut self.documentation_groups,
                    cancelled,
                )?);
            } else {
                included_ranges(node, children, &mut ranges);
            }
        }
        canonicalize(&mut ranges);
        ranges = intersect(&ranges, parent_ranges);
        if ranges.is_empty() {
            return Ok(());
        }
        let Some(language) = detected else {
            self.unknown.push((ranges, source));
            return Ok(());
        };
        let injection = Injection {
            language,
            ranges,
            combined,
            source,
        };
        if let Some(index) = self
            .found
            .iter()
            .position(|existing| existing.ranges == injection.ranges)
        {
            if self.found[index].source <= source {
                self.found[index] = injection;
            }
        } else {
            self.found.push(injection);
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<Injection> {
        let unknown = &self.unknown;
        // An explicit unknown fence language must not resurrect a shebang fallback.
        self.found.retain(|injection| {
            !unknown
                .iter()
                .any(|(ranges, source)| injection.source < *source && injection.ranges == *ranges)
        });
        let mut result: Vec<Injection> = Vec::new();
        for injection in self.found {
            if let Some(existing) = result.iter_mut().find(|existing| {
                existing.combined && injection.combined && existing.language == injection.language
            }) {
                existing.ranges.extend(injection.ranges);
                canonicalize(&mut existing.ranges);
            } else {
                result.push(injection);
            }
        }
        result
    }
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

/// A child's syntax nodes can span gaps in the parent's included ranges.
/// Intersect before parsing, not just when publishing spans: prefixes must not
/// become tokens in a nested language.
fn intersect(ranges: &[Range], parent: &[Range]) -> Vec<Range> {
    let mut result = Vec::new();
    let mut first_parent = 0;
    for range in ranges {
        while first_parent < parent.len() && parent[first_parent].end_byte <= range.start_byte {
            first_parent += 1;
        }
        for outer in &parent[first_parent..] {
            if outer.start_byte >= range.end_byte {
                break;
            }
            let start = if range.start_byte >= outer.start_byte {
                range
            } else {
                outer
            };
            let end = if range.end_byte <= outer.end_byte {
                range
            } else {
                outer
            };
            if start.start_byte < end.end_byte {
                result.push(Range {
                    start_byte: start.start_byte,
                    start_point: start.start_point,
                    end_byte: end.end_byte,
                    end_point: end.end_point,
                });
            }
        }
    }
    result
}
