//! Compiled queries, shared per language (0111 decision 3). Compiling a
//! highlight query costs tens of milliseconds; every injected block of a
//! language used to pay it again. A query is immutable once compiled and
//! tree-sitter marks it `Send + Sync`, so each language compiles at most
//! once per process and every highlighter — host or injected, on any
//! thread — borrows the same `'static` copy. Highlighters own only their
//! parser and tree.
use crate::languages::{self, LanguageId};
use crate::spans::{CaptureStyle, Emphasis};
use crate::Class;
use std::sync::OnceLock;
use tree_sitter::{Language, Query};

pub(crate) struct Compiled {
    pub language: Language,
    pub highlights: Query,
    /// Capture index → class, resolved once.
    pub classes: Vec<CaptureStyle>,
    /// Injection patterns scoped to the requested window.
    pub windowed: Option<Query>,
    /// `injection.combined` patterns. A combined injection is one parse
    /// over all of its ranges in the document, so these patterns run over
    /// the whole tree: a closing tag keeps its colour when its opener is
    /// off-screen (0111 decision 4).
    pub combined: Option<Query>,
}

/// One slot per registry entry, filled on first use. A grammar whose
/// query fails to compile stays `None` for the process: the failure is
/// as deterministic as the vendored query text.
fn table() -> &'static [OnceLock<Option<Compiled>>] {
    static TABLE: OnceLock<Vec<OnceLock<Option<Compiled>>>> = OnceLock::new();
    TABLE.get_or_init(|| (0..languages::count()).map(|_| OnceLock::new()).collect())
}

pub(crate) fn compiled(id: LanguageId) -> Option<&'static Compiled> {
    table()
        .get(languages::index(id))?
        .get_or_init(|| compile(id))
        .as_ref()
}

fn compile(id: LanguageId) -> Option<Compiled> {
    let spec = id.spec();
    let highlights = Query::new(&spec.language, spec.highlights).ok()?;
    let classes = highlights
        .capture_names()
        .iter()
        .map(|name| CaptureStyle {
            class: Class::from_capture(name),
            emphasis: Emphasis::from_capture(name),
        })
        .collect();
    let (windowed, combined) = if spec.injections.is_empty() {
        (None, None)
    } else {
        // Two copies of one query, each with the other half's patterns
        // disabled: pattern indices (and so property settings) agree.
        let mut windowed = Query::new(&spec.language, spec.injections).ok()?;
        let mut combined = Query::new(&spec.language, spec.injections).ok()?;
        let mut any_windowed = false;
        let mut any_combined = false;
        for pattern in 0..windowed.pattern_count() {
            let is_combined = windowed
                .property_settings(pattern)
                .iter()
                .any(|property| property.key.as_ref() == "injection.combined");
            if is_combined {
                windowed.disable_pattern(pattern);
                any_combined = true;
            } else {
                combined.disable_pattern(pattern);
                any_windowed = true;
            }
        }
        (
            any_windowed.then_some(windowed),
            any_combined.then_some(combined),
        )
    };
    Some(Compiled {
        language: spec.language,
        highlights,
        classes,
        windowed,
        combined,
    })
}
