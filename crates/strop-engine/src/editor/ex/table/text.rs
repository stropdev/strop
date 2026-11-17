//! Commands over the buffer's lines: `:s :g :norm :d :y :m :t :j :pu :>
//! :< :sort :! :@ :r`, undo, marks and registers.

use super::{name, Arg, ExCommand, Family, Name, Run};
use crate::editor::ex::*;

const fn c(names: &'static [Name], usage: &'static str, doc: &'static str, run: Run) -> ExCommand {
    super::command(names, usage, doc, run).family(Family::Text)
}

const SORT_FLAGS: &[(&str, &str)] = &[
    ("u", "keep only the first of equal lines"),
    ("n", "by the first number on the line"),
    ("i", "ignore case"),
];

pub(super) const ROWS: &[ExCommand] = &[
    c(
        &[name("substitute", 1)],
        ":s/pat/new/[flags]",
        "replace matches, with a live preview (flags g i I c n &)",
        ex_substitute,
    )
    .lines()
    .edits()
    .arg(Arg::Pattern),
    c(
        &[name("&", 1)],
        ":&[&]",
        "repeat the last :s (:&& keeps its flags)",
        ex_substitute,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("~", 1)],
        ":~",
        "repeat the last :s with the last search pattern",
        ex_substitute,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("global", 1)],
        ":g[!]/pat/cmd",
        "run cmd on every matching line (! on the others)",
        ex_global,
    )
    .lines()
    .bang()
    .arg(Arg::Command),
    c(
        &[name("vglobal", 1)],
        ":v/pat/cmd",
        "run cmd on every line that does not match",
        ex_global,
    )
    .lines()
    .arg(Arg::Command),
    c(
        &[name("normal", 4)],
        ":norm[al] {keys}",
        "type Normal-mode keys on each line of the range",
        ex_normal_command,
    )
    .lines()
    .bang()
    .arg(Arg::Text),
    c(
        &[name("delete", 1)],
        ":d [x] [count]",
        "delete lines (into register x)",
        ex_delete,
    )
    .lines()
    .edits()
    .arg(Arg::Register),
    c(
        &[name("yank", 1)],
        ":y [x] [count]",
        "yank lines (into register x)",
        ex_yank,
    )
    .lines()
    .arg(Arg::Register),
    c(
        &[name("move", 1)],
        ":m {address}",
        "move lines below the address",
        ex_move,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("t", 1), name("copy", 2)],
        ":t {address}",
        "copy lines below the address",
        ex_copy,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("join", 1)],
        ":j[!] [count]",
        "join lines (! keeps the white space)",
        ex_join,
    )
    .lines()
    .bang()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("put", 2)],
        ":pu[!] [x]",
        "put register x below the line (! above)",
        ex_put,
    )
    .lines()
    .bang()
    .edits()
    .arg(Arg::Register),
    c(
        &[name(">", 1)],
        ":> [count]",
        "indent lines (:>> twice)",
        ex_shift,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("<", 1)],
        ":< [count]",
        "unindent lines (:<< twice)",
        ex_shift,
    )
    .lines()
    .edits()
    .arg(Arg::Text),
    c(
        &[name("sort", 3)],
        ":sor[t][!] [u][n][i]",
        "sort lines (! reversed)",
        ex_sort,
    )
    .lines()
    .bang()
    .edits()
    .arg(Arg::Words(SORT_FLAGS)),
    c(
        &[name("!", 1)],
        ":[range]!cmd",
        "run a shell command; with a range, filter the lines through it",
        ex_shell,
    )
    .lines()
    .arg(Arg::Shell),
    c(
        &[name("@", 1)],
        ":@: :@x",
        "repeat the last command line / run register x as commands",
        ex_at,
    )
    .arg(Arg::Register),
    c(
        &[name("read", 1)],
        ":r !cmd",
        "insert the command's output below the cursor",
        |e, c| e.read_command(c.arg),
    )
    .arg(Arg::Shell),
    c(&[name("undo", 1)], ":u[ndo]", "undo one change", |e, _| {
        e.undo()
    })
    .key("u"),
    c(
        &[name("redo", 3)],
        ":red[o]",
        "redo one undone change",
        |e, _| e.redo(),
    )
    .key("ctrl-r"),
    c(
        &[name("k", 1), name("mark", 2)],
        ":k {a}",
        "set mark a on the line",
        ex_mark,
    )
    .lines()
    .arg(Arg::Mark),
    c(&[name("marks", 5)], ":marks", "list the marks", |e, _| {
        open_marks_listing(e)
    }),
    c(
        &[name("delmarks", 4)],
        ":delm[arks][!] {a b c-e}",
        "delete marks (! every lowercase mark)",
        ex_delmarks,
    )
    .bang()
    .arg(Arg::Mark),
    c(
        &[name("registers", 3), name("display", 2)],
        ":reg [names]",
        "list the registers",
        |e, c| open_registers_listing(e, c.arg),
    )
    .arg(Arg::Register),
    c(
        &[name("nohlsearch", 3)],
        ":noh[lsearch]",
        "clear the persistent search highlight",
        |e, _| {
            // nohlsearch: the persistent highlight drops (0001 §5.8)
            e.last_search = None;
        },
    ),
];
