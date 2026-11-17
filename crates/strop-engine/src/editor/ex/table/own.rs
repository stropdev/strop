//! strop's own commands: terminals, themes, remotes, the language
//! server, collections, occurrence selection, indentation, recovery.

use super::{name, Arg, ExCommand, Family, Name, Run};

const fn c(names: &'static [Name], usage: &'static str, doc: &'static str, run: Run) -> ExCommand {
    super::command(names, usage, doc, run).family(Family::Own)
}

const REMOTE_WORDS: &[(&str, &str)] = &[
    ("root", "open the host's root folder"),
    ("home", "open the host's home folder"),
    ("connect", "connect to a host: :remote connect URI"),
    ("disconnect", "close a host's connection"),
    ("clear", "close every connection"),
    ("list", "list the connections in a buffer"),
];

const TAB_SIZES: &[(&str, &str)] = &[
    ("auto", "the configured or detected width"),
    ("2", "two columns"),
    ("4", "four columns"),
    ("8", "eight columns"),
];

const INDENT_STYLES: &[(&str, &str)] = &[
    ("spaces", "indent with spaces"),
    ("tabs", "indent with tabs"),
    ("auto", "the configured or detected style"),
];

const RECOVER_WORDS: &[(&str, &str)] = &[
    ("list", "list the recoverable drafts"),
    ("restore", "restore draft N"),
    ("discard", "discard draft N"),
];

pub(super) const ROWS: &[ExCommand] = &[
    c(
        &[name("terminal", 2)],
        ":ter[minal] [cmd]",
        "an interactive shell in the selected namespace",
        |e, c| {
            e.launch_terminal(c.arg, false);
            Ok(())
        },
    )
    .arg(Arg::Shell),
    c(
        &[name("terminal-local", 14)],
        ":terminal-local [cmd]",
        "a local shell, even from an SSH context",
        |e, c| {
            e.launch_terminal(c.arg, true);
            Ok(())
        },
    )
    .arg(Arg::Shell),
    c(
        &[name("theme", 5)],
        ":theme [name]",
        "choose a built-in interface palette",
        |e, c| {
            e.theme_command(c.arg);
            Ok(())
        },
    )
    .arg(Arg::Theme),
    c(
        &[name("filter", 6)],
        ":filter [text]",
        "narrow the folder to matching names; alone it clears",
        |e, c| {
            if let Err(error) = e.filter_directory(c.arg.to_owned()) {
                e.message = error;
            }
            Ok(())
        },
    )
    .arg(Arg::Text),
    c(
        &[name("remote", 6)],
        ":remote [root|home|connect|disconnect|clear|list]",
        "choose a remote destination, or manage connections",
        |e, c| {
            e.run_remote_ex(c.name, c.arg);
            Ok(())
        },
    )
    .arg(Arg::Words(REMOTE_WORDS)),
    c(
        &[name("trust", 5)],
        ":trust",
        "authorize the pending endpoint or project command",
        |e, _| {
            e.request_trust();
            Ok(())
        },
    ),
    c(
        &[name("explain", 7)],
        ":explain",
        "why: workspaces, LSP readiness/failures, effective config",
        |e, _| {
            e.open_explain();
            Ok(())
        },
    ),
    c(
        &[name("format", 6)],
        ":format",
        "format the buffer through the language server",
        |e, _| {
            e.lsp_format();
            Ok(())
        },
    ),
    c(
        &[name("rename", 6)],
        ":rename {new}",
        "rename the symbol under the cursor",
        |e, c| {
            if c.arg.is_empty() {
                e.message = ":rename needs a new name".into();
            } else {
                e.lsp_rename(c.arg);
            }
            Ok(())
        },
    )
    .arg(Arg::Text),
    c(
        &[name("symbols", 7)],
        ":symbols",
        "the document symbols picker",
        |e, _| {
            e.lsp_document_symbols_pub();
            Ok(())
        },
    ),
    c(
        &[name("copen", 4), name("cwindow", 2)],
        ":cope[n]",
        "show the collection, on its current hit (the quickfix list)",
        |e, _| {
            e.quickfix_open();
            Ok(())
        },
    ),
    c(
        &[name("cnext", 2)],
        ":cn[ext]",
        "next hit of the collection",
        |e, _| {
            e.quickfix_step(true);
            Ok(())
        },
    )
    .key("]q"),
    c(
        &[name("cprevious", 2), name("cNext", 2)],
        ":cp[revious]",
        "previous hit of the collection",
        |e, _| {
            e.quickfix_step(false);
            Ok(())
        },
    )
    .key("[q"),
    c(
        &[name("cc", 2)],
        ":cc [N]",
        "hit N of the collection",
        |e, c| {
            e.quickfix_goto(c.arg);
            Ok(())
        },
    )
    .arg(Arg::Text),
    c(
        &[name("cdo", 3)],
        ":cdo {cmd}",
        "run a command on every hit line of the collection",
        |e, c| {
            e.quickfix_do(c.arg, false);
            Ok(())
        },
    )
    .arg(Arg::Command),
    c(
        &[name("cfdo", 3)],
        ":cfdo {cmd}",
        "run a command once in every file of the collection",
        |e, c| {
            e.quickfix_do(c.arg, true);
            Ok(())
        },
    )
    .arg(Arg::Command),
    c(
        &[name("select-next", 11)],
        ":select-next",
        "select the next occurrence of the word or selection",
        |e, _| {
            e.occurrence_next_pub();
            Ok(())
        },
    )
    .key("gb"),
    c(
        &[name("select-all", 10)],
        ":select-all",
        "select every occurrence in the buffer",
        |e, _| {
            e.occurrence_all_pub();
            Ok(())
        },
    )
    .key("gB"),
    c(
        &[name("select-skip", 11)],
        ":select-skip",
        "skip the next occurrence candidate",
        |e, _| {
            e.occurrence_skip_pub();
            Ok(())
        },
    ),
    c(
        &[name("select-pop", 10)],
        ":select-pop",
        "drop the last-added occurrence",
        |e, _| {
            e.occurrence_pop_pub();
            Ok(())
        },
    ),
    c(
        &[name("tab-size", 8)],
        ":tab-size [N|auto]",
        "indentation width for this file (alone: a selector)",
        |e, c| {
            e.tab_size_command(c.arg);
            Ok(())
        },
    )
    .arg(Arg::Words(TAB_SIZES)),
    c(
        &[name("indent-style", 12)],
        ":indent-style [spaces|tabs|auto]",
        "indent with tabs or spaces in this file",
        |e, c| {
            e.indent_style_command(c.arg);
            Ok(())
        },
    )
    .arg(Arg::Words(INDENT_STYLES)),
    c(
        &[name("recover", 7)],
        ":recover [list|restore N|discard N]",
        "recover unsaved text after a crash",
        |e, c| {
            e.run_recovery_ex(c.arg);
            Ok(())
        },
    )
    .arg(Arg::Words(RECOVER_WORDS)),
];
