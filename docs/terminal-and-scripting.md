# Terminals

Strop's terminal is Neovim's: a real PTY in a real buffer, one escape chord,
and a NORMAL mode that is just NORMAL mode.

| Keys | Effect |
| --- | --- |
| `Space t` | new shell in a horizontal split below, in TERMINAL mode |
| `Space T` | new shell in the current pane (= `:terminal`) |
| `:terminal [CMD]`, `:term` | new shell, or run `CMD` and keep its output |
| `Ctrl-\ Ctrl-N` | TERMINAL → NORMAL (the only escape chord) |
| `i` `I` `a` `A` | NORMAL → TERMINAL at the child's cursor |
| `:q` | hide a running terminal; `Space b` returns to it |
| `:bd!` | stop the child and delete the buffer |
| `:qa` | refuses while terminals run (one notice); `:qa!` stops them and quits |
| `:r !CMD` | insert a command's output below the cursor (vim) |

**TERMINAL mode** sends every key to the child — Esc, Ctrl-C, Ctrl-W, Alt
and function keys included. The mode chip reads `TERMINAL`; the modeline
shows the one way out. Legacy terminals deliver `Ctrl-\` as the byte
`0x1c` (what vim calls `Ctrl-4`); it is the same chord. Pastes go through
as every terminal sends them: bracketed when the child enabled bracketed
paste, raw bytes otherwise.

**NORMAL mode** is a live, read-only buffer of the screen and its
scrollback: ordinary motions, search, selection and yank work on it, and
output keeps landing while you read. The view follows output while the
cursor is on the last line — `k`, `Ctrl-U` or a search stop following,
`G` resumes it, exactly as in Neovim. Positions map by stream byte, so a
cursor or mark stays on the same text when scrollback drops rows.

Buffers are named `term://{directory}//{N}:{program}` (`N` numbers sessions
from 1 per editor) in `Space b`, pane titles and the modeline. Switching
away and back (`Ctrl-^`, `Space b`, pane focus) restores the cursor and
viewport and lands in NORMAL mode. An exited session stays readable with
its exit status; `:terminal` starts a new one.

The shell is `$SHELL` (or `/bin/sh`) in the current directory, running in
the editor process. In an SSH directory with an admitted worker the
session runs on that host as the remote user's login shell (`$SHELL -l`)
and keeps the endpoint in its name; SSH without an admitted worker
refuses. **Open local terminal** / `:terminal-local` is a deliberate
local choice, never a fallback against a remote-looking path. For a
container, run `docker exec -it <container> sh` in a terminal. The
displayed launch directory is the one selected at launch and does not
follow `cd`.

Isolation: child screen/clipboard/window commands cannot control the
outer terminal; clipboard reads and writes are denied (your own yank is
separate authority). `:w!` and `:set noro` cannot turn a terminal into a
writable file — yank into an ordinary buffer to export text. Closing a
split does not kill its process. Deliberately detached processes outside
the owned session can outlive it.

Terminal buffers use bounded history, input and immutable snapshots. One
TERMINAL-mode view controls PTY geometry; mirrors never independently
resize it. Linux supervision requires pidfds. Source builds require
**Zig 0.16.0** explicitly installed for the pinned static emulator;
prebuilt users need neither Zig nor a Ghostty installation. See
[plan 0090 §4](../plans/0090-one-editor.md).

# Headless scripts and traces

`strop headless SCRIPT [+LINE] [FILE[:LINE]]` uses the same editor and
service handlers (`--script SCRIPT` and `--headless SCRIPT` are
equivalents; see [cli.md](cli.md)). A script is one directive per line;
empty lines and `#` comments are ignored:

| Directive | Arguments | Meaning |
| --- | --- | --- |
| `buffer` | `JSON_STRING` | initial buffer text; must be the first directive |
| `keys` | `KEYS` | literal keys and chords: `<esc>` `<cr>` `<c-r>` `<a-x>` `<f5>` |
| `key` | `JSON_KEY` | one semantic editor key, e.g. `"Esc"` or `{"Char":"x"}` |
| `input` | `JSON_INPUT` | one physical key/text/paste event; preserves modifiers and release/repeat |
| `paste` | `JSON_STRING` | one bracketed-paste event, e.g. `"first\nsecond"` |
| `resize` | `COLS ROWS` | resize the terminal cell grid |
| `frame` | | print the current rendered cell grid |
| `state` | | print editor state as JSON |
| `settle` | `[MS]` | return when finite work drains; timeout fails (default 30000 ms) |
| `wait` | `MS` | deliberately wait this long while processing events |
| `quit-intent` | | send the GUI close request (not a Ctrl-C editor shortcut) |

JSON strings use double quotes and JSON escapes; `keys` text is not
quoted. `key` is an already-normalized semantic editor command, not
terminal input.

```text
keys :e source.rs<cr>
settle 5000
keys /needle<cr>
frame
state
```

`settle [MS]` returns when finite owned work drains (default 30,000 ms) and
exits nonzero on timeout. `wait MS` remains a deliberate delay. Pure
grammar work keeps key, paste, repeat and macro ordering; filesystem and
service jobs require an explicit settle before a script relies on their
result. An interactive shell is not an eternally pending job: terminal
settle covers admission/transport/frame barriers, not arbitrary command
execution. Script completion explicitly closes and drains its owned
terminal sessions.

Headless `:trust` uses the same explicit per-project/endpoint consent and
`XDG_STATE_HOME` (or `$HOME/.local/state`) store as the TUI. It does not
restore or automatically persist editor sessions. Project configuration
never grants itself permission to execute commands.

## Reporting a bug

```sh
strop --log-file issue.jsonl path/to/file.rs
# Include sensitive documents, service results and rendered editor cells:
strop --log-file issue-full.jsonl --log-content path/to/file.rs
# Explicitly opt into private terminal commands, keys, paste and output:
strop --log-file terminal-full.jsonl --log-terminal-content
strop headless steps.keys path/to/file.rs --log-file headless.jsonl
strop replay issue-full.jsonl
strop export issue-full.jsonl > issue-metadata.jsonl
# Optional input-only extraction; inspect before executing:
strop replay --script issue-full.jsonl > replay.keys
```

Attach the JSONL file with the observed and expected behavior. Logs contain
keys, commands, paths and diagnostic messages; full-content logs also
contain documents, pastes, service results and rendered cells. **Inspect
before sharing.** Existing files are never overwritten. Use the recording
version of strop for full replay: it requires a complete capture and does
not repeat native side effects. The input-only script is different: it can
run commands again and does not reproduce external service results.

Terminal content is excluded even by ordinary `--log-content`. Entering a
private terminal boundary records an opaque marker and leaves the remaining
capture metadata-only; replay refuses the omitted history.
`--log-terminal-content` is the separate explicit consent, visibly marked
**REC**. Terminal captures use full `strop replay`, not input-only
extraction, which cannot reproduce PTY state.

Captures are bounded to 64 MiB, 100,000 events and 256 KiB per record.
Hitting a limit is explicit and an incomplete trace is refused for full
replay. Metadata export retains only event categories and sequence
numbers, not arbitrary payloads; the export is deliberately not replayable.

See [tracing design](../plans/0029-session-tracing.md).
