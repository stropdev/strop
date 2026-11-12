//! The ex vocabulary: one row per command. Dispatch looks a typed word
//! up here, completion lists these names, and the keymap test checks
//! each one renders in `?` — one list, no drift.

use super::ExCall;
use crate::editor::ex::*;
use crate::editor::Editor;

/// What a range before the command means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Addr {
    /// The command takes none; typing one is an error.
    None,
    /// Buffer lines, checked against the buffer before the command runs.
    Lines,
    /// A number that is not a line (`:3b`).
    Count,
}

pub(crate) struct ExCommand {
    /// The full name; completion offers it.
    pub name: &'static str,
    /// The shortest abbreviation Vim accepts, in bytes (`:s`, `:norm`).
    pub min: usize,
    pub range: Addr,
    /// Whether `!` may follow the name.
    pub bang: bool,
    /// The command changes the buffer's text: a readonly buffer refuses.
    pub edits: bool,
    pub doc: &'static str,
    pub run: fn(&mut Editor, &ExCall<'_>),
}

type Run = fn(&mut Editor, &ExCall<'_>);

const fn command(name: &'static str, min: usize, doc: &'static str, run: Run) -> ExCommand {
    ExCommand {
        name,
        min,
        range: Addr::None,
        bang: false,
        edits: false,
        doc,
        run,
    }
}

/// A command only its full name reaches (strop's own vocabulary).
const fn own(name: &'static str, doc: &'static str, run: Run) -> ExCommand {
    command(name, name.len(), doc, run)
}

impl ExCommand {
    const fn lines(mut self) -> Self {
        self.range = Addr::Lines;
        self
    }
    const fn count(mut self) -> Self {
        self.range = Addr::Count;
        self
    }
    const fn bang(mut self) -> Self {
        self.bang = true;
        self
    }
    const fn edits(mut self) -> Self {
        self.edits = true;
        self
    }
}

/// The command a typed word names: the first row it abbreviates.
pub(crate) fn lookup(word: &str) -> Option<&'static ExCommand> {
    COMMANDS
        .iter()
        .find(|command| word.len() >= command.min && command.name.starts_with(word))
}

/// The command with exactly this name.
pub(crate) fn exact_name(word: &str) -> Option<&'static ExCommand> {
    COMMANDS.iter().find(|command| command.name == word)
}

fn optional(arg: &str) -> Option<&str> {
    (!arg.is_empty()).then_some(arg)
}

/// Rows are in Vim's precedence order where abbreviations overlap
/// (`:s` is substitute, not `:set`); otherwise grouped by family.
pub(crate) const COMMANDS: &[ExCommand] = &[
    // -- text ------------------------------------------------------
    command(
        "substitute",
        1,
        ":s/pat/new/[flags]: replace matches (g i I c n &)",
        ex_substitute,
    )
    .lines()
    .edits(),
    command(
        "&",
        1,
        "repeat the last :s (:&& keeps its flags)",
        ex_substitute,
    )
    .lines()
    .edits(),
    command(
        "~",
        1,
        "repeat the last :s with the last search pattern",
        ex_substitute,
    )
    .lines()
    .edits(),
    command(
        "global",
        1,
        ":g/pat/cmd: run cmd on every matching line (:g! on the others)",
        ex_global,
    )
    .lines()
    .bang(),
    command(
        "vglobal",
        1,
        ":v/pat/cmd: run cmd on every line that does not match",
        ex_global,
    )
    .lines(),
    command(
        "normal",
        4,
        ":norm keys: type Normal-mode keys on each line of the range",
        ex_normal_command,
    )
    .lines()
    .bang(),
    command(
        "delete",
        1,
        ":d [x] [count]: delete lines (into register x)",
        ex_delete,
    )
    .lines()
    .edits(),
    command(
        "yank",
        1,
        ":y [x] [count]: yank lines (into register x)",
        ex_yank,
    )
    .lines(),
    command(
        "move",
        1,
        ":m {address}: move lines below the address",
        ex_move,
    )
    .lines()
    .edits(),
    command(
        "t",
        1,
        ":t {address}: copy lines below the address",
        ex_copy,
    )
    .lines()
    .edits(),
    command(
        "copy",
        2,
        ":co {address}: copy lines below the address",
        ex_copy,
    )
    .lines()
    .edits(),
    command(
        "join",
        1,
        ":j[!] [count]: join lines (! keeps the white space)",
        ex_join,
    )
    .lines()
    .bang()
    .edits(),
    command(
        "put",
        2,
        ":pu[!] [x]: put register x below (! above) the line",
        ex_put,
    )
    .lines()
    .bang()
    .edits(),
    command(">", 1, ":> [count]: indent lines (:>> twice)", ex_shift)
        .lines()
        .edits(),
    command("<", 1, ":< [count]: unindent lines (:<< twice)", ex_shift)
        .lines()
        .edits(),
    command(
        "sort",
        3,
        ":sort[!] [u][n][i]: sort lines (! reversed)",
        ex_sort,
    )
    .lines()
    .bang()
    .edits(),
    command(
        "!",
        1,
        ":!cmd runs a shell command; :{range}!cmd filters the lines through it",
        ex_shell,
    )
    .lines(),
    command(
        "@",
        1,
        ":@: repeats the last command line; :@x runs register x as commands",
        ex_at,
    ),
    command(
        "read",
        1,
        ":r !cmd inserts the command's output below the cursor",
        |e, c| e.read_command(c.arg),
    ),
    command("undo", 1, "undo one change", |e, _| e.undo()),
    command("redo", 3, "redo one undone change", |e, _| e.redo()),
    command("k", 1, ":k a: set mark a on the line", ex_mark).lines(),
    command("mark", 2, ":mark a: set mark a on the line", ex_mark).lines(),
    command("marks", 5, "list the marks", |e, _| open_marks_listing(e)),
    command(
        "delmarks",
        4,
        ":delm a b c-e: delete marks (! every lowercase mark)",
        ex_delmarks,
    )
    .bang(),
    command(
        "registers",
        3,
        ":reg [names]: list the registers",
        |e, c| open_registers_listing(e, c.arg),
    ),
    command("display", 2, ":di [names]: list the registers", |e, c| {
        open_registers_listing(e, c.arg)
    }),
    command(
        "nohlsearch",
        3,
        "clear the persistent search highlight",
        |e, _| {
            // nohlsearch: the persistent highlight drops (0001 §5.8)
            e.last_search = None;
        },
    ),
    // -- buffers ---------------------------------------------------
    command(
        "buffer",
        1,
        ":b {n|name}: go to a buffer by number or unique name fragment",
        ex_buffer,
    )
    .count()
    .bang(),
    command("bnext", 2, "go to the next buffer", |e, c| {
        ex_buffer_step(e, c, true)
    })
    .count()
    .bang(),
    command("bprevious", 2, "go to the previous buffer", |e, c| {
        ex_buffer_step(e, c, false)
    })
    .count()
    .bang(),
    command("bNext", 2, "go to the previous buffer", |e, c| {
        ex_buffer_step(e, c, false)
    })
    .count()
    .bang(),
    command(
        "bdelete",
        2,
        ":bd[!] [n]: delete a buffer (! discards changes, stops a terminal)",
        ex_buffer_delete,
    )
    .count()
    .bang(),
    command("ls", 2, "list the buffers", |e, c| {
        open_buffer_listing(e, c.bang)
    })
    .bang(),
    command("buffers", 7, "list the buffers", |e, c| {
        open_buffer_listing(e, c.bang)
    })
    .bang(),
    command("files", 5, "list the buffers", |e, c| {
        open_buffer_listing(e, c.bang)
    })
    .bang(),
    // -- files and windows -----------------------------------------
    command(
        "write",
        1,
        ":w[!] [file]: write (! over a file changed on disk)",
        ex_write,
    )
    .bang(),
    command("wall", 2, "write every modified buffer", |e, c| {
        write_all(e, c.bang, false)
    })
    .bang(),
    command("wq", 2, "write and quit", ex_write).bang(),
    command(
        "wqall",
        3,
        "write every modified buffer and quit",
        |e, c| write_all(e, c.bang, true),
    )
    .bang(),
    command("xall", 2, "write every modified buffer and quit", |e, c| {
        write_all(e, c.bang, true)
    })
    .bang(),
    command("xit", 1, "write if modified, then quit", ex_write).bang(),
    command("exit", 3, "write if modified, then quit", ex_write).bang(),
    command("update", 2, "write if modified", ex_write).bang(),
    command(
        "quit",
        1,
        ":q[!]: close the window, or the buffer in the last one",
        |e, c| e.close_pane_or_buffer(c.bang),
    )
    .bang(),
    command(
        "qall",
        2,
        "quit all (fails on unsaved; :qa! discards)",
        |e, c| e.quit_all(c.bang),
    )
    .bang(),
    command(
        "quitall",
        5,
        "quit all (fails on unsaved; :quita! discards)",
        |e, c| e.quit_all(c.bang),
    )
    .bang(),
    command(
        "close",
        3,
        "close the window (never the last one)",
        |e, _| ex_close(e),
    ),
    command("only", 2, "close every other window", |e, _| ex_only(e)),
    command(
        "edit",
        1,
        ":e[!] [file]: edit a file, or reload this one",
        ex_edit,
    )
    .bang(),
    command("enew", 3, "edit a new, unnamed buffer", |e, _| {
        ex_new(e, None)
    }),
    command("new", 3, "split with a new, unnamed buffer", |e, _| {
        ex_new(e, Some(false))
    }),
    command(
        "vnew",
        3,
        "split vertically with a new, unnamed buffer",
        |e, _| ex_new(e, Some(true)),
    ),
    command("view", 3, "open readonly", ex_view),
    command("split", 2, "split horizontal", |e, c| {
        e.split(false, optional(c.arg))
    }),
    command("vsplit", 2, "split vertical", |e, c| {
        e.split(true, optional(c.arg))
    }),
    command(
        "cd",
        2,
        ":cd [dir]: change the project root (- goes back)",
        ex_cd,
    ),
    command(
        "pwd",
        2,
        "show the project root (Files, Search, terminals)",
        |e, _| {
            e.message = e.cwd.display().to_string();
        },
    ),
    command("set", 2, "readonly option: ro/noro", ex_set),
    command("help", 1, "help buffer", |e, c| e.open_help_topic(c.arg)),
    command(
        "messages",
        3,
        "message history: the last 50 echo-row messages",
        |e, _| e.open_messages(),
    ),
    command("jumps", 2, "open the jumplist picker", |e, _| {
        e.open_jumps_picker()
    }),
    // -- strop's own -----------------------------------------------
    command(
        "terminal",
        2,
        "interactive shell in the selected namespace",
        |e, c| e.launch_terminal(c.arg, false),
    ),
    own(
        "terminal-local",
        "explicit local shell even from an SSH context",
        |e, c| e.launch_terminal(c.arg, true),
    ),
    own("theme", "choose a built-in interface palette", |e, c| {
        e.theme_command(c.arg)
    }),
    own("fs", "filesystem actions on the listed files", |e, c| {
        e.run_filesystem_ex(c.arg, c.range.map(super::range::LineRange::lines))
    })
    .lines(),
    own(
        "browse",
        "browse remote directory: :browse [URI]",
        |e, c| {
            if let Err(error) = e.browse_directory(c.arg.trim()) {
                e.message = error;
            }
        },
    ),
    own(
        "filter",
        "filter remote directory by literal filename",
        |e, c| {
            if let Err(error) = e.filter_directory(c.arg.to_owned()) {
                e.message = error;
            }
        },
    ),
    own(
        "remote",
        "choose remote destination; root/home/connect/disconnect/clear/list",
        |e, c| {
            e.run_remote_ex(c.name, c.arg);
        },
    ),
    own(
        "trust",
        "authorize the pending endpoint/project command",
        |e, _| e.request_trust(),
    ),
    own(
        "explain",
        "why: workspaces, LSP readiness/failures, effective config",
        |e, _| e.open_explain(),
    ),
    own(
        "format",
        "format the buffer through the language server",
        |e, _| e.lsp_format(),
    ),
    own(
        "rename",
        "rename the symbol under the cursor: :rename NEW",
        |e, c| {
            if c.arg.is_empty() {
                e.message = ":rename needs a new name".into();
            } else {
                e.lsp_rename(c.arg);
            }
        },
    ),
    own("symbols", "open the document symbols picker", |e, _| {
        e.lsp_document_symbols_pub()
    }),
    command(
        "copen",
        4,
        "show the collection, on its current hit (the quickfix list)",
        |e, _| e.quickfix_open(),
    ),
    command(
        "cwindow",
        2,
        "show the collection, on its current hit (the quickfix list)",
        |e, _| e.quickfix_open(),
    ),
    command("cnext", 2, "next hit of the collection (]q)", |e, _| {
        e.quickfix_step(true)
    }),
    command(
        "cprevious",
        2,
        "previous hit of the collection ([q)",
        |e, _| e.quickfix_step(false),
    ),
    command("cNext", 2, "previous hit of the collection ([q)", |e, _| {
        e.quickfix_step(false)
    }),
    command("cc", 2, ":cc [N]: hit N of the collection", |e, c| {
        e.quickfix_goto(c.arg)
    }),
    command(
        "cdo",
        3,
        "run an ex command on every hit line of the collection: :cdo s/a/b/g",
        |e, c| e.quickfix_do(c.arg, false),
    ),
    command(
        "cfdo",
        3,
        "run an ex command once in every file of the collection: :cfdo w",
        |e, c| e.quickfix_do(c.arg, true),
    ),
    own(
        "select-next",
        "select the next occurrence of the word or selection",
        |e, _| e.occurrence_next_pub(),
    ),
    own("select-all", "select all occurrences in buffer", |e, _| {
        e.occurrence_all_pub()
    }),
    own(
        "select-skip",
        "skip the next occurrence candidate",
        |e, _| e.occurrence_skip_pub(),
    ),
    own("select-pop", "drop the last-added occurrence", |e, _| {
        e.occurrence_pop_pub()
    }),
    own("tab-size", "choose indentation width: N or auto", |e, c| {
        e.tab_size_command(c.arg)
    }),
    own(
        "indent-style",
        "choose indentation style: spaces, tabs or auto",
        |e, c| e.indent_style_command(c.arg),
    ),
    own(
        "recover",
        "recover unsaved text after a crash: list, restore N, discard N",
        |e, c| e.run_recovery_ex(c.arg),
    ),
];
