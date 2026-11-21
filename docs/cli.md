# The strop command line

`strop FILE` opens a file. This page lists the other commands and the
options. `strop --help` prints the same list (in colour on a terminal,
plain when piped).

```text
Usage: strop [OPTIONS] [FILE[:LINE] | DIR | ssh://URL | -]
```

With no command, strop opens the file or directory, or an empty buffer.
`strop` alone in a project brings back the files you had open there (see
[Sessions](editor.md#sessions)). `strop -` reads standard input
([Reading stdin](editor.md#reading-stdin)).

Use `--` to open a file whose name would otherwise be a command or option:
`strop -- compat` edits the file `compat`. Commands such as `strop compat`
and `strop config` do not take an editor file or stdin operand.

## Commands

| Command | What it does |
| --- | --- |
| `strop remote URL` | open an `ssh://` file or directory for editing |
| `strop headless SCRIPT [FILE]` | run a script of keys against a file, with no screen |
| `strop replay LOG` | show a recorded session again (`--script` prints its keys as a script) |
| `strop export LOG` | strip a recording down to event types, safe to share |
| `strop config` | show the config file and its current values |
| `strop update` | update strop itself (`--check` only checks) |
| `strop bench [SCENARIO]` | run the built-in speed tests |
| `strop compat` | print the list of Vim keys strop supports |
| `strop help` | print help |

Scripts, recordings and replay are described in
[scripting.md](scripting.md).

## Options

| Option | Meaning |
| --- | --- |
| `-R`, `--readonly` | open the initial file read-only, including in `headless` and at remote locations |
| `+LINE` | initial line for FILE (or use `FILE:LINE`) |
| `--theme NAME` | colour theme for this run (see [Themes](editor.md#themes)) |
| `--log[=PATH]` | record this session to a file, for a bug report (`STROP_LOG=PATH` works too; without PATH the file is `strop-log.jsonl`) |
| `--log-content` | include file text in the recording (sensitive) |
| `--log-terminal-content` | also what you type and see in terminals |
| `-h`, `--help` / `-V`, `--version` | help and version |

`--log-file PATH` is `--log=PATH` with the path as its own argument. Both
use the exact filename, including non-UTF-8 bytes.

## Remote locations

```text
ssh://[user@]host[:port]/absolute/path[:LINE]
```

`strop remote ssh://user@devbox:2222/etc/app.conf:120` opens that file
at line 120; a plain `strop ssh://…` operand does the same. Once connected you can edit and save; `-R` opens read-only. To
read a growing log, pipe it in: `ssh devbox tail -f /var/log/app.log | strop -`.
Details are in [remote.md](remote.md).

## Exit codes

`0` on success. Otherwise nonzero, with a one-line `strop: …` explanation
on stderr.
