# The strop CLI

One binary, subcommands for the occasional jobs, flags for everything else.
`strop --help` prints the same reference live (styled when stdout is a
terminal, plain when piped).

```text
Usage: strop [OPTIONS] [FILE[:LINE] | DIR | ssh://URL]
```

With no subcommand, strop opens the file, directory, or empty buffer —
the common case stays `strop file.rs`.

## Commands

| Command | What it does |
| --- | --- |
| `strop edit [FILE]` | open a file, directory, or an empty buffer (the default) |
| `strop remote URL` | open an `ssh://` file or directory for editing |
| `strop headless SCRIPT [FILE]` | run the scripted driver over a file |
| `strop replay TRACE` | replay a session trace (`--script` extracts a driver script) |
| `strop export TRACE` | export a trace's metadata (privacy-safe, not replayable) |
| `strop ui-stdio` | UI protocol backend on stdin/stdout |
| `strop config` | show the config file and its current values |
| `strop update` | self-update (`--check` only checks) |
| `strop bench [SCENARIO]` | run the performance-gate scenarios |
| `strop licenses` | built-in theme attributions |
| `strop compat` | dump the vim-compatibility table |
| `strop help` | print help |

Every command also has its original flag spelling (`--licenses`,
`--replay-script`, `--ui-stdio`, …); scripts and muscle memory keep working.

## Options

| Option | Meaning |
| --- | --- |
| `-R`, `--readonly` | open read-only |
| `+LINE` | initial line for FILE (or use `FILE:LINE`) |
| `--theme NAME` | session-start palette (see [Themes](editor.md#themes)) |
| `--remote URL` | shorthand for `strop remote URL` |
| `--follow` | follow a growing remote file |
| `--tail BYTES` | remote view: the last BYTES |
| `--range A:B` | remote view: one byte range |
| `--script PATH` | headless driver script (same as `headless`) |
| `--log[=PATH]` | record a session trace (`STROP_LOG` works too) |
| `--log-content` | include file text in the trace (sensitive) |
| `--log-terminal-content` | also terminal keys and output |
| `-h`, `--help` / `-V`, `--version` | help and version |

## Remote locations

```text
ssh://[user@]host[:port]/absolute/path[:LINE]
```

`strop --remote "ssh://tnawara@123.space:4343/etc/app.conf:120"` and
`strop remote ssh://tnawara@123.space:4343/etc/app.conf:120` are the same
command. Remote files open read-only by default; `:remote edit` inside the
editor grants write authority. The full remote contract lives in
[remote.md](remote.md).

## Exit codes

`0` on success, nonzero with a one-line `strop: …` explanation on stderr
otherwise. Worker mode (`--worker-stdio`) is internal: stdout is protocol
frames from the first byte and it takes no other arguments.
