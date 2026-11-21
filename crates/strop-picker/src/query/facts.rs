//! What a filter judges about one row (0109 §3.2d): the facts a list
//! row carries, owned once when the row is built and borrowed by every
//! pass. Filtering never formats a row, clones a path or splits text.

use std::ops::Range;
use std::sync::Arc;

use strop_core::symbol_kind::SymbolKind;

use crate::{Item, Payload};

/// One row as a filter sees it.
#[derive(Debug, Clone, Copy, Default)]
pub struct RowFacts<'a> {
    /// Workspace-relative path.
    pub path: Option<&'a str>,
    /// 1-based line.
    pub line: Option<usize>,
    pub kind: Option<SymbolKind>,
    /// What content atoms match: a symbol's name, a hit's line.
    pub text: &'a str,
    /// A second text content atoms may match: a symbol's container.
    pub container: Option<&'a str>,
}

/// The facts an [`Item`] carries beyond its text and payload. Rows that
/// are not files or symbols leave it empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ItemFacts {
    /// Workspace-relative path, for rows whose payload names none or an
    /// absolute one. Rows of one file share one allocation.
    pub path: Option<Arc<str>>,
    pub kind: Option<SymbolKind>,
    /// The part of the text content atoms match; all of it when absent.
    pub subject: Option<Range<u32>>,
    /// The part of the text naming a symbol's container.
    pub container: Option<Range<u32>>,
}

impl Item {
    /// The row as a filter sees it, borrowed.
    pub fn facts(&self) -> RowFacts<'_> {
        let slice = |range: &Range<u32>| self.text.get(range.start as usize..range.end as usize);
        let (path, line) = match &self.payload {
            Payload::File(path) => (path.to_str(), None),
            Payload::Grep { line, .. } | Payload::Remote { line, .. } => (None, Some(*line)),
            Payload::Symbol { line, .. } => (None, Some(line.get() + 1)),
            _ => (None, None),
        };
        RowFacts {
            path: self.facts.path.as_deref().or(path),
            line,
            kind: self.facts.kind,
            text: self
                .facts
                .subject
                .as_ref()
                .and_then(slice)
                .unwrap_or(&self.text),
            container: self.facts.container.as_ref().and_then(slice),
        }
    }
}

/// One declaration as a list row: `name  container detail`, where
/// `detail` is what the list adds (a path, a line), written straight
/// into the row's one text allocation.
pub struct SymbolRow<'a> {
    pub name: &'a str,
    pub container: Option<&'a str>,
    pub detail: std::fmt::Arguments<'a>,
    pub kind: SymbolKind,
    /// Workspace-relative path, shared by the rows of one file.
    pub path: Option<Arc<str>>,
    pub payload: Payload,
}

impl Item {
    /// The one symbol-row shape (0109): the kind is the chip and a fact,
    /// and filters see the name and the container, not the decoration.
    pub fn symbol(row: SymbolRow<'_>) -> Item {
        use std::fmt::Write;
        let container = row.container.filter(|container| !container.is_empty());
        let mut text =
            String::with_capacity(row.name.len() + 3 + container.map_or(0, str::len) + 16);
        text.push_str(row.name);
        text.push_str("  ");
        let container = container.map(|container| {
            let start = text.len() as u32;
            text.push_str(container);
            text.push(' ');
            start..start + container.len() as u32
        });
        // writing into a String cannot fail
        let _ = text.write_fmt(row.detail);
        Item {
            badge: Some(std::borrow::Cow::Borrowed(row.kind.chip())),
            text,
            payload: row.payload,
            facts: ItemFacts {
                path: row.path,
                kind: Some(row.kind),
                subject: Some(0..row.name.len() as u32),
                container,
            },
        }
    }
}
