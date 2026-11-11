# Scripts and bug reports

Run strop without a screen from a script of keys, and record a session to
attach to a bug report.

## Running a script

`strop headless SCRIPT [+LINE] [FILE[:LINE]]` runs the real editor with no
screen, on a 100×30 grid, and feeds it the script. `strop --script SCRIPT`
is the same command (see [cli.md](cli.md)). A script is one directive per
line; empty lines and lines starting with `#` are ignored:

| Directive | Arguments | Meaning |
| --- | --- | --- |
| `buffer` | `JSON_STRING` | the text to start with; must be the first directive |
| `keys` | `KEYS` | keys as you would type them: `<esc>` `<cr>` `<c-r>` `<a-x>` `<f5>` |
| `key` | `JSON_KEY` | one editor key, e.g. `"Esc"` or `{"Char":"x"}` |
| `input` | `JSON_INPUT` | one exact key, text or paste event, with its modifiers |
| `paste` | `JSON_STRING` | one paste, e.g. `"first\nsecond"` |
| `resize` | `COLS ROWS` | change the screen size |
| `frame` | | print the screen as text |
| `state` | | print the editor's state as JSON |
| `settle` | `[MS]` | wait until background work is finished; fails after MS (default 120000) |
| `wait` | `MS` | wait this long |
| `quit-intent` | | quit at once, as when the terminal window is closed |

JSON strings use double quotes and JSON escapes; the text after `keys` is
not quoted. `keys` is what you would type. `key` and `input` are for tests
that need one exact key event; most scripts never need them. Keys for a
program inside a terminal must be sent with `keys` or `input`.

```text
keys :e source.rs<cr>
settle 5000
keys /needle<cr>
frame
state
```

`settle [MS]` waits until background work has finished: file loads, saves,
searches, git, language-server replies. It fails the script, with a nonzero
exit status, if that takes longer than MS milliseconds (120,000 by
default). Keys, pastes, repeats and macros always run in order and need no
`settle`. Anything that reads a file or asks a language server does: put a
`settle` before the script depends on the result. A terminal counts as
settled once the shell has started and its output so far is drawn. `settle`
does not wait for a command you ran in it; use `wait MS` for that. When the
script ends, its terminals are closed.

`quit-intent` does not ask about unsaved buffers or running terminals.
After the editor has quit, any further directive except `settle` fails the
script.

A project can name its own language-server command, and strop runs it only
after you allow that project with [`:trust`](editor.md#trust). Scripts
follow the same rule and use the same saved answers. A headless run does
not load or save a session.

## Reporting a bug

```sh
strop --log=issue.jsonl path/to/file.rs
# Also record file text, language-server replies and what was on screen:
strop --log=issue-full.jsonl --log-content path/to/file.rs
# Also record what you type and see inside terminals:
strop --log=terminal-full.jsonl --log-terminal-content
strop headless steps.keys path/to/file.rs --log=headless.jsonl
strop replay issue-full.jsonl
strop export issue-full.jsonl > issue-safe.jsonl
# Optional: the keys as a script. Read it before you run it:
strop replay --script issue-full.jsonl > replay.keys
```

Attach the JSONL file and say what happened and what you expected. A log
holds your keys, commands, paths and messages. A log made with
`--log-content` also holds file text, pastes, language-server replies and
what was on screen. **Read it before you share it.** strop never overwrites
an existing log file: pick a new name. `--log` alone writes
`strop-log.jsonl` in the current directory, and `STROP_LOG=PATH` in the
environment does the same as `--log=PATH`.

`strop replay` shows the session again without touching your files or
running any command, and prints the state it ends in. It needs a complete
log made with `--log-content`, and the strop version that made it; a
version that records differently refuses the log and asks you to record
again. `strop replay --script` is different. It gives you the keys as a
script. Running that script really does run the commands again, and a
language server may answer differently, so read it first. From a log made
without `--log-content` the script has the keys but not the file text; run
it against the original file (`strop headless replay.keys FILE`).

What you type and see in a terminal is never recorded by `--log-content`.
From the moment you open a terminal, the log keeps only event types, with
no text, and can no longer be replayed. To record terminal content as well,
use `--log-terminal-content`; the status line shows **REC** in a terminal
while it is on. A log with terminal content can be replayed with
`strop replay` but not turned into a script.

A recording stops at 512 MiB or 100,000 events, and a single event is
limited to 256 KiB. The editor keeps running; the log is marked incomplete
and `strop replay` refuses it. `strop export` keeps only what kind of event
happened and in what order. It has no text, paths or keys, so it is safe to
share, and it cannot be replayed.
