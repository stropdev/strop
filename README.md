<div align="center">
  <img src="logo.svg" width="480" alt="strop — an editor card mid ci[, the pending cut tinted, a razor peeking over the edge">
  <h3>see the cut before you make it.</h3>
  <p>Neovim's hands · Helix's spine · rootle's eyes · GitLens' memory</p>
  <p>
    <a href="https://strop.dev">strop.dev</a> ·
    <a href="https://github.com/stropdev/strop/releases">what's new</a> ·
    <a href="https://crates.io/crates/strop-editor">crates.io</a>
  </p>
</div>

![strop demo](demos/demo.gif)

**strop** is a modal text editor in Rust with Vim-style commands, project search,
git surfaces and operator previews. Composition windows such as `d/pattern`
preview their affected ranges; completed object chords such as `ci[` execute
at the completing key rather than pausing for an inspection step.

## Install

```sh
curl -fsSL https://strop.dev/install.sh | sh     # prebuilt static binary (linux/macOS)
brew install stropdev/tap/strop                  # homebrew (formula; macOS cask is prebuilt)
cargo install strop-editor --locked              # crates.io
mise use cargo:strop-editor                      # mise
```

Already installed? `strop update` self-updates tarball installs.

## The grammar in one card

```
h j k l w b e 0 $ gg G %      motions          i a A o O            insert
d y c > < + motion/object     operators        dd yy cc D C Y s x   shortcuts
iw i" i' i( i[ i{             text objects     f t / ? n N          find & search
v V                           visual           u ctrl-r .           undo, redo, repeat
"a … "+                       registers        :w :q :e :help        ex line
Q                            multicursor      Space c              cursor below

Space  f files · o remote · b buffers · / grep · R replace · ? help   C-w … panes
Space g  l log · h history · b blame gutter · y/o permalink · u/s/p hunk
```

Operator previews and execution consume the same resolver. Surround
(`ys`/`cs`/`ds`), multicursor cascades (`Q`/`Space c`), project-wide search &
replace with live row previews, per-project sessions, and undo
history that crosses restarts (with a `Space u` tree browser), tree-sitter highlighting
for 23 languages (including CMake, Markdown, TOML, YAML, HTML and CSS), git gutter +
blame + commit dive chains, SHA-resolved permalinks over OSC52, and helix-flavored
`.strop/languages.toml` LSP config.

Search supports a bounded Vim-magic regex dialect (and `\v` very magic), including
collections, groups, alternation, repetition and backreferences. Unsupported
constructs report an error instead of being treated as literals. Search previews
and Enter use the same counted resolver. Horizontal views, block selection and
carets share display-cell geometry; new edits preserve the buffer's line endings.

Markdown includes inline emphasis, links, tables and fenced-language highlighting;
HTML embeds JavaScript and CSS. Grammars and licensed queries ship in the binary.
Syntax, structural indentation guides and search counts are revision-owned worker
results. Long-line byte/cell checkpoints keep horizontal navigation and typing
from repeatedly walking the line prefix.

File/native work runs on owned jobs rather than blocking keystrokes. Sessions
use private atomic files and lossless native paths, including undo history.

The modeline keeps filenames, live status and position legible at narrow widths.
Git history uses quieter metadata, clear file hierarchy and native-path-safe
navigation; see the [modeline and Git polish](plans/0032-modeline-and-git-polish.md).

## Read-only SSH workspaces

```sh
strop +120 ssh://user@devbox/var/log/app.log
strop --tail 65536 ssh://devbox/var/log/app.log
strop --range 1048576:65536 ssh://devbox/var/log/app.log
strop --follow ssh://devbox/var/log/app.log
strop ssh://devbox/~/project/src/lib.rs
strop ssh://devbox/etc/
```

These are real read-only buffers: motions, `/`/`?`, visual selection, yank and
splits work unchanged. Escape cancels an open or stops following; `:e!` refreshes
the snapshot. Follow sticks to EOF only while you stay there. Shrink, replacement
and changed overlap produce a visible reset; size/mtime alone are not file identity.
The modeline identifies partial byte windows, whose line numbers are window-relative.

Inside strop:

Press **Space o** or run **`:remote`** to choose a known/connected destination or
select **Add a host**. Enter `host`, `user@host:port`, or a complete SSH path.
Enter is the connection action; opening the chooser does not authenticate.
New hosts start at `/`; successful directory choices are remembered privately.
`:remote home` uses negotiated remote-home expansion; `:remote root` returns to `/`.

```vim
:tail 65536 ssh://devbox/var/log/app.log
:range 1048576 65536 ssh://devbox/var/log/app.log
:follow
:unfollow
:browse ssh://devbox/var/log/
:filter app
:remote connect ssh://devbox
:remote list
:remote disconnect ssh://devbox
:remote clear
```

Directory listings are searchable real buffers with kind, POSIX permissions,
server-reported byte size and native filename columns. Missing attributes show
`?`, distinct from mode `000` and size zero. Directory sizes are not recursive
totals. Links and special entries have distinct type markers. Enter opens an entry;
`-`, Backspace or `../` returns to the parent and restores the selected child.
`:filter` narrows names, and an empty filter restores the full listing.
Tab completes SSH hosts and paths without starting authentication: remote candidates
need a live authorized connection or cached data. Connections are shared by endpoint
and held by documents or explicit `:remote connect` pins; clear/disconnect retires
them without touching external SSH masters.

Full-file snapshots support remote LSP diagnostics, hover, definition/references and
source/header navigation, plus Git context, staged/unstaged diffs, log, blame and
commit/file navigation. Services run **on the remote host**, never against a local
lookalike path. Project-command trust is scoped to the endpoint and remote root
(`:trust`). Partial windows and following refuse full-document language services;
remote writes, Git mutations and shell/filter commands remain unsupported.

OpenSSH supplies aliases, keys, agent and ProxyJump configuration. Host keys must
already be trusted; authentication is noninteractive. Percent-encode reserved path
bytes (`%20`, `%23`, `%25`); native Unix filenames stay intact. Reads are bounded to
256 MiB of UTF-8 text; range/tail edges exclude incomplete UTF-8 characters. Home
expansion requires the server's `expand-path@openssh.com` extension.

SFTP reading needs no remote Python or daemon setup. Remote Git/LSP additionally
need a POSIX execution environment and Git/the selected language server.
Compatible Python 3.8+ is discovered as `python3` or a versioned program on remote
PATH; set local `STROP_REMOTE_PYTHON=/opt/tools/python3.11` to select an explicit
remote executable. An invalid override fails rather than choosing a fallback.
Owned process groups are cleaned up when the server observes lease loss; network
partitions delay detection, descendants creating new sessions can escape the group,
and a killed supervisor cannot guarantee cleanup. Remote content is not persisted
or automatically restored.

The standalone `strop-remote` crate owns transport and execution; editor glue owns
views and replay. See the [workspace contract](plans/0036-remote-workspace-execution.md),
[protocol evidence](plans/0034-ssh-log-buffers.md),
[prioritized remote roadmap](plans/0035-remote-workflow-roadmap.md), and
[Dev Containers design](plans/0037-devcontainers-and-workspace-contexts.md).

## Headless scripts

`strop --headless SCRIPT [+LINE] [FILE[:LINE]]` uses the same editor and service
handlers. `--script SCRIPT` is equivalent. `strop --help` lists every directive:
`buffer`, `keys`, `key`, `paste`, `resize`, `frame`, `state`, `settle`, `wait` and
`quit-intent`. JSON strings use double quotes and JSON escapes; `keys` text is
unquoted and accepts tokens such as `<esc>`, `<cr>`, `<bs>` and `<lt>`.

```text
keys :e source.rs<cr>
settle 5000
keys /needle<cr>
frame
state
```

`settle [MS]` returns when finite owned work drains (default 30,000 ms) and exits
nonzero on timeout. `wait MS` remains a deliberate delay. Pure grammar work keeps
key, paste, repeat and macro ordering; filesystem/service jobs require an explicit
settle before a script relies on their result.

Headless `:trust` uses the same explicit per-project/endpoint consent and
`XDG_STATE_HOME` (or `$HOME/.local/state`) store as the TUI. It does not restore or
automatically persist editor sessions. Project configuration never grants itself
permission to execute commands.

## Reporting a bug

```sh
strop --log-file issue.jsonl path/to/file.rs
# Include sensitive documents, service results and terminal observations:
strop --log-file issue-full.jsonl --log-content path/to/file.rs
strop --headless steps.keys path/to/file.rs --log-file headless.jsonl
strop --replay issue-full.jsonl
strop --export-metadata issue-full.jsonl > issue-metadata.jsonl
# Optional input-only extraction; inspect before executing:
strop --replay-script issue-full.jsonl > replay.keys
```

Attach the JSONL file with the observed and expected behavior. Logs contain keys,
commands, paths and diagnostic messages; full-content logs also contain documents,
pastes, service results and rendered cells. **Inspect before sharing.** Existing
files are never overwritten. Use the recording version of strop for full replay:
it requires a complete capture and does not repeat native side effects.
The input-only script is different: it can
run commands again and does not reproduce external service results.

Captures are bounded to 64 MiB, 100,000 events and 256 KiB per record. Hitting a
limit is explicit and an incomplete trace is refused for full replay. Metadata
export retains only event categories and sequence numbers, not arbitrary payloads;
the export is deliberately not replayable.

See [tracing design](plans/0029-session-tracing.md), the
[prioritized review/roadmap](plans/0028-roadmap-and-review.md), and the
[P1/P2 execution contract](plans/0031-p1-p2-execution.md).

## Links

- [strop.dev](https://strop.dev) — demo, palettes, install
- [plans/](plans/) — the numbered design contracts everything answers to

MIT license · © 2026 [Tarek Nawara](https://github.com/tknawara)
