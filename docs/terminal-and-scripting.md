# Local terminals

`:terminal` opens the shell selected by `SHELL` (or `/bin/sh`) in a real
local PTY. `:terminal COMMAND` runs an explicit command. Directory
**Terminal here** (`Space a` / `:fs terminal`) uses the selected local
directory without changing the editor's cwd. SSH/container contexts refuse
interactive execution; the separately labelled **Open local terminal** /
`:terminal-local` is a deliberate local choice, never a fallback against a
remote-looking path.

- **Terminal input:** Esc, Ctrl-C, Ctrl-R, Ctrl-W, Alt and function keys go
  to the child. **F2** enters editor Normal mode directly, without an
  OS-sensitive chord; **Ctrl-\ Ctrl-N** remains an alternative (legacy
  terminals may spell the prefix as Ctrl-4). The modeline names both paths
  and expands a pending chord. Legacy Esc/Alt ambiguity remains an
  outer-terminal limitation. After entering Normal, **Space w** owns pane
  commands.
- **Normal inspection:** ordinary motions, search, selection and yank use a
  pinned, read-only logical text snapshot. Output continues without moving
  that snapshot; **i**, **I**, **a** or **A** follows live input again.
  Other inspecting panes keep their snapshot. An exited session stays
  readable and never restarts implicitly.
- **Paste:** bracketed paste follows the child's negotiated mode.
  Otherwise, multiline/control input is held for explicit consent: enter
  Normal mode, then `:terminal-paste` to confirm or
  `:terminal-paste-cancel` to discard. Native paste encoding normalizes
  terminal control bytes; no extra Enter is appended.
- **Lifetime:** `:terminal-stop` stops the owned session and retains
  output. Closing a split does not kill its process; hidden sessions remain
  in `Space b`. `:qa` refuses live sessions; `:qa!` stops and drains them.
  Deliberately detached processes outside the owned terminal session can
  outlive it.
- **Isolation:** child screen/clipboard/window commands cannot control the
  outer terminal. Clipboard reads/writes are denied; explicit user
  selection/yank is separate authority. `:w!` and `:set noro` cannot turn a
  terminal into a writable file binding — yank into an ordinary buffer to
  export text.

Terminal buffers use bounded history, input and immutable snapshots. One
focused input view controls PTY geometry; mirrors never independently
resize it. Linux supervision requires pidfds. Source builds require
**Zig 0.16.0** explicitly installed for the pinned static emulator;
prebuilt users need neither Zig nor a Ghostty installation. See the
[terminal contract](../plans/0055-embedded-terminal-tui-and-gui.md).

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
