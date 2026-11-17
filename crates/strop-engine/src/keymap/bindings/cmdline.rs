//! The command line: how it opens and what a range is. The commands
//! themselves are not listed here — `?` writes one row per command from
//! the registry (`editor/ex/table`), so the two can never drift.
use super::super::{Binding, Handler};

const fn row(keys: &'static str, desc: &'static str, id: &'static str) -> Binding {
    Binding {
        keys,
        desc,
        sections: &["cmdline"],
        live: true,
        id,
        handler: Handler::TextLine,
    }
}

pub(super) const ROWS: &[Binding] = &[
    row(
        ":",
        "the command line (from Visual it opens with '<,'>)",
        "cmdline-open",
    ),
    row(
        ":N :N,M :. :$ :% :'a :'<,'> :/pat/ :?pat? :.+N :N;+M",
        "ranges: lines, marks, searches and offsets; :N alone goes to the line",
        "cmdline-ranges",
    ),
];
