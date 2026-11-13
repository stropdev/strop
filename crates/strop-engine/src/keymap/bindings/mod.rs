//! The binding table, one file per section (0088 A7) in the curated
//! `SECTIONS` order; `BINDINGS` is their concatenation, so dispatch,
//! help and which-key keep reading one flat table.
use super::{Binding, Handler};

mod editing;
mod ex;
mod folder;
mod git;
mod insert;
mod leader;
mod moving;
mod panes;
mod pickers;
mod search;
mod terminal;
mod visual;

const TOTAL: usize = moving::ROWS.len()
    + editing::ROWS.len()
    + insert::ROWS.len()
    + visual::ROWS.len()
    + search::ROWS.len()
    + leader::ROWS.len()
    + panes::ROWS.len()
    + git::ROWS.len()
    + folder::ROWS.len()
    + pickers::ROWS.len()
    + terminal::ROWS.len()
    + ex::ROWS.len();

const PLACEHOLDER: Binding = Binding {
    keys: "",
    desc: "",
    sections: &[],
    live: false,
    id: "",
    handler: Handler::Soon,
};

const fn concat(parts: &[&[Binding]]) -> [Binding; TOTAL] {
    let mut out = [PLACEHOLDER; TOTAL];
    let mut at = 0;
    let mut part = 0;
    while part < parts.len() {
        let rows = parts[part];
        let mut row = 0;
        while row < rows.len() {
            out[at] = rows[row];
            at += 1;
            row += 1;
        }
        part += 1;
    }
    out
}

/// Every binding, section by section.
pub static BINDINGS: &[Binding] = &concat(&[
    moving::ROWS,
    editing::ROWS,
    insert::ROWS,
    visual::ROWS,
    search::ROWS,
    leader::ROWS,
    panes::ROWS,
    git::ROWS,
    folder::ROWS,
    pickers::ROWS,
    terminal::ROWS,
    ex::ROWS,
]);
