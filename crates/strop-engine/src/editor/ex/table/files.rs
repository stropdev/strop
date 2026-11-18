//! Files and windows: `:w :q :e :sp :vs :cd :set :help :messages …`.

use super::{name, optional, Arg, ExCommand, Family, Name, Run};
use crate::editor::ex::*;

const fn c(names: &'static [Name], usage: &'static str, doc: &'static str, run: Run) -> ExCommand {
    super::command(names, usage, doc, run).family(Family::Files)
}

const SET_WORDS: &[(&str, &str)] = &[
    ("readonly", "refuse edits to this buffer"),
    ("noreadonly", "allow edits again"),
    ("ro", "refuse edits to this buffer"),
    ("noro", "allow edits again"),
];

pub(super) const ROWS: &[ExCommand] = &[
    c(
        &[name("write", 1)],
        ":w[!] [file]",
        "write the buffer (! over a file changed on disk)",
        ex_write,
    )
    .bang()
    .arg(Arg::File),
    c(
        &[name("wall", 2)],
        ":wa[ll][!]",
        "write every modified buffer",
        |e, c| write_all(e, c.bang, false),
    )
    .bang(),
    c(
        &[name("wq", 2)],
        ":wq[!] [file]",
        "write and quit",
        ex_write,
    )
    .bang()
    .arg(Arg::File),
    c(
        &[name("wqall", 3), name("xall", 2)],
        ":wqa[ll][!]",
        "write every modified buffer and quit",
        |e, c| write_all(e, c.bang, true),
    )
    .bang(),
    c(
        &[name("xit", 1), name("exit", 3)],
        ":x[it][!] [file]",
        "write if modified, then quit",
        ex_write,
    )
    .bang()
    .arg(Arg::File),
    c(
        &[name("update", 2)],
        ":up[date][!] [file]",
        "write if modified",
        ex_write,
    )
    .bang()
    .arg(Arg::File),
    c(
        &[name("quit", 1)],
        ":q[uit][!]",
        "close the window; the tab page when it is the last window in it; the buffer when it is the last window",
        |e, c| {
            e.close_pane_or_buffer(c.bang);
            Ok(())
        },
    )
    .bang(),
    c(
        &[name("qall", 2), name("quitall", 5)],
        ":qa[ll][!]",
        "quit all (fails on unsaved; ! discards)",
        |e, c| {
            e.quit_all(c.bang);
            Ok(())
        },
    )
    .bang(),
    c(
        &[name("close", 3)],
        ":clo[se]",
        "close the window (never the last one)",
        |e, _| ex_close(e),
    )
    .key("ctrl-w c"),
    c(
        &[name("only", 2)],
        ":on[ly]",
        "close every other window",
        |e, _| ex_only(e),
    )
    .key("ctrl-w o"),
    c(
        &[name("edit", 1)],
        ":e[dit][!] [file]",
        "edit a file, or reload this one",
        ex_edit,
    )
    .bang()
    .arg(Arg::File),
    c(
        &[name("enew", 3)],
        ":ene[w]",
        "edit a new, unnamed buffer",
        |e, _| ex_new(e, None),
    ),
    c(
        &[name("new", 3)],
        ":new",
        "split with a new, unnamed buffer",
        |e, _| ex_new(e, Some(false)),
    )
    .key("ctrl-w n"),
    c(
        &[name("vnew", 3)],
        ":vne[w]",
        "split vertically with a new, unnamed buffer",
        |e, _| ex_new(e, Some(true)),
    ),
    c(
        &[name("view", 3)],
        ":vie[w] {file}",
        "open a file readonly",
        ex_view,
    )
    .arg(Arg::File),
    c(
        &[name("split", 2)],
        ":sp[lit] [file]",
        "split below",
        |e, c| {
            e.split(false, optional(c.arg));
            Ok(())
        },
    )
    .arg(Arg::File)
    .key("ctrl-w s"),
    c(
        &[name("vsplit", 2)],
        ":vs[plit] [file]",
        "split to the right",
        |e, c| {
            e.split(true, optional(c.arg));
            Ok(())
        },
    )
    .arg(Arg::File)
    .key("ctrl-w v"),
    c(
        &[name("cd", 2)],
        ":cd [dir|-]",
        "change the project root (- goes back)",
        ex_cd,
    )
    .arg(Arg::Directory),
    c(
        &[name("pwd", 2)],
        ":pw[d]",
        "show the project root (Files, Search, terminals)",
        |e, _| {
            e.message = e.cwd.display().to_string();
            Ok(())
        },
    ),
    c(
        &[name("set", 2)],
        ":se[t] {ro|noro}",
        "make this buffer readonly, or writable again",
        ex_set,
    )
    .arg(Arg::Words(SET_WORDS)),
    c(
        &[name("help", 1)],
        ":h[elp] [topic]",
        "the help buffer (this text; / searches it)",
        |e, c| {
            e.open_help_topic(c.arg);
            Ok(())
        },
    )
    .arg(Arg::HelpTopic),
    c(
        &[name("messages", 3)],
        ":mes[sages]",
        "the last 50 echo-row messages",
        |e, _| {
            e.open_messages();
            Ok(())
        },
    ),
    c(
        &[name("jumps", 2)],
        ":ju[mps]",
        "the jumplist picker",
        |e, _| {
            e.open_jumps_picker();
            Ok(())
        },
    )
    .key("space j"),
];
