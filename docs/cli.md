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

## Commands

| Command | What it does |
| --- | --- |
| `strop edit [FILE]` | open a file, directory, or an empty buffer (the default) |
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
| `-R`, `--readonly` | open read-only |
| `+LINE` | initial line for FILE (or use `FILE:LINE`) |
| `--theme NAME` | colour theme for this run (see [Themes](editor.md#themes)) |
| `--remote URL` | same as `strop remote URL` |
| `--script PATH` | same as `strop headless PATH` |
| `--log[=PATH]` | record this session to a file, for a bug report (`STROP_LOG=PATH` works too; without PATH the file is `strop-log.jsonl`) |
| `--log-content` | include file text in the recording (sensitive) |
| `--log-terminal-content` | also what you type and see in terminals |
| `-h`, `--help` / `-V`, `--version` | help and version |

Older spellings still work: `--log-file PATH` for `--log=PATH`, and
`--headless SCRIPT` for `--script SCRIPT`.

## Remote locations

```text
ssh://[user@]host[:port]/absolute/path[:LINE]
```

`strop --remote "ssh://user@devbox:2222/etc/app.conf:120"` and
`strop remote ssh://user@devbox:2222/etc/app.conf:120` are the same
command. Once connected you can edit and save; `-R` opens read-only. To
read a growing log, pipe it in: `ssh devbox tail -f /var/log/app.log | strop -`.
Details are in [remote.md](remote.md).

## Exit codes

`0` on success. Otherwise nonzero, with a one-line `strop: …` explanation
on stderr.
