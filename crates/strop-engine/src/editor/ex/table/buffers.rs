//! Buffers: `:b :bn :bp :bd :ls`.

use super::{name, Arg, ExCommand, Family, Name, Run};
use crate::editor::ex::*;

const fn c(names: &'static [Name], usage: &'static str, doc: &'static str, run: Run) -> ExCommand {
    super::command(names, usage, doc, run).family(Family::Buffers)
}

pub(super) const ROWS: &[ExCommand] = &[
    c(
        &[name("buffer", 1)],
        ":b {number|name}",
        "go to a buffer by number or unique name fragment",
        ex_buffer,
    )
    .count()
    .bang()
    .arg(Arg::Buffer),
    c(
        &[name("bnext", 2)],
        ":bn[ext]",
        "go to the next buffer",
        |e, c| ex_buffer_step(e, c, true),
    )
    .count()
    .bang(),
    c(
        &[name("bprevious", 2), name("bNext", 2)],
        ":bp[revious]",
        "go to the previous buffer",
        |e, c| ex_buffer_step(e, c, false),
    )
    .count()
    .bang(),
    c(
        &[name("bdelete", 2)],
        ":bd[elete][!] [number]",
        "delete a buffer (! discards changes, stops a terminal)",
        ex_buffer_delete,
    )
    .count()
    .bang()
    .arg(Arg::Buffer),
    c(
        &[name("ls", 2), name("buffers", 7), name("files", 5)],
        ":ls[!]",
        "list the buffers",
        |e, c| open_buffer_listing(e, c.bang),
    )
    .bang(),
];
