//! Tab pages (plans/0113): `:tabn :tabp :tabfir :tabl :tabnew :tab :tabc
//! :tabo :tabm`. Row order is precedence: `:tabne` is `:tabnext`, as in
//! Vim, because `tabnext` (from 4) comes before `tabnew` (from 6).

use super::{name, Arg, ExCommand, Family, Name, Run};
use crate::editor::ex::*;

const fn c(names: &'static [Name], usage: &'static str, doc: &'static str, run: Run) -> ExCommand {
    super::command(names, usage, doc, run).family(Family::Tabs)
}

pub(super) const ROWS: &[ExCommand] = &[
    c(
        &[name("tabnext", 4)],
        ":tabn[ext] [N]",
        "go to the next tab page, or to tab page N",
        ex_tab_next,
    )
    .count()
    .key("gt"),
    c(
        &[name("tabprevious", 4), name("tabNext", 4)],
        ":tabp[revious] [N]",
        "go to the previous tab page, or N back",
        ex_tab_previous,
    )
    .count()
    .key("gT"),
    c(
        &[name("tabfirst", 6), name("tabrewind", 4)],
        ":tabfir[st]",
        "go to the first tab page",
        |e, _| ex_tab_end(e, false),
    ),
    c(
        &[name("tablast", 4)],
        ":tabl[ast]",
        "go to the last tab page",
        |e, _| ex_tab_end(e, true),
    ),
    c(
        &[name("tabnew", 6), name("tabedit", 4)],
        ":tabnew [file]",
        "open a new tab page after this one",
        ex_tab_new,
    )
    .arg(Arg::File),
    c(
        &[name("tab", 3)],
        ":tab {split|new}",
        "open the split in a new tab page; :tab split shows this buffer alone",
        ex_tab,
    )
    .arg(Arg::Command),
    c(
        &[name("tabclose", 4)],
        ":tabc[lose][!] [N]",
        "close this tab page, or tab page N; its buffers stay open",
        ex_tab_close,
    )
    .count()
    .bang(),
    c(
        &[name("tabonly", 4)],
        ":tabo[nly][!]",
        "close every other tab page",
        ex_tab_only,
    )
    .bang(),
    c(
        &[name("tabmove", 4)],
        ":tabm[ove] [N]",
        "move this tab page after tab page N (0 first, $ or nothing last, +N -N relative)",
        ex_tab_move,
    ),
];
