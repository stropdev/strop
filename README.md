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

Space  f files · b buffers · / grep · R replace · ? help   C-w …    panes
Space g  l log · h history · b blame gutter · y/o permalink · u/s/p hunk
```

Operator previews and execution consume the same resolver. Surround
(`ys`/`cs`/`ds`), multicursor cascades (`Q`/`Space c`), project-wide search &
replace with live row previews, per-project sessions, and undo
history that crosses restarts (with a `Space u` tree browser), tree-sitter highlighting
for thirteen languages (bash/fish/lua/sql included, shebang detection too), git gutter +
blame + commit dive chains, SHA-resolved permalinks over OSC52, and helix-flavored
`.strop/languages.toml` LSP config.

Search supports a bounded Vim-magic regex dialect (and `\v` very magic), including
collections, groups, alternation, repetition and backreferences. Unsupported
constructs report an error instead of being treated as literals. Search previews
and Enter use the same counted resolver. Horizontal views, block selection and
carets share display-cell geometry; new edits preserve the buffer's line endings.

File/native work runs on owned jobs rather than blocking keystrokes. Sessions
use private atomic files and lossless native paths, including undo history.

The modeline keeps filenames, live status and position legible at narrow widths.
Git history uses quieter metadata, clear file hierarchy and native-path-safe
navigation; see the [modeline and Git polish](plans/0032-modeline-and-git-polish.md).

## SSH log buffers

```sh
strop +120 ssh://user@devbox/var/log/app.log
```

Open the same URI with `:e` or `:view`. It becomes a real read-only buffer:
normal motions, `/`/`?`, visual selection, yank and splits work unchanged.
Escape cancels an in-flight open; `:e!` refreshes the current remote snapshot.
OpenSSH supplies your aliases, keys, agent and ProxyJump configuration. Host keys
must already be trusted; authentication is noninteractive. Paths are absolute;
percent-encode reserved bytes (`%20`, `%23`, `%25`). Native Unix filename bytes
stay intact. Snapshots are bounded to 256 MiB and must be valid UTF-8 text.

The standalone `strop-remote` crate owns transport/identity; editor glue owns views
and replay. No local LSP/Git service attaches to a remote snapshot. See the
[SSH contract and model evidence](plans/0034-ssh-log-buffers.md),
[full remote workflow roadmap](plans/0035-remote-workflow-roadmap.md), and
[Dev Containers design](plans/0037-devcontainers-and-workspace-contexts.md).
Tail/follow, directory browsing and remote workspace services are the separately
tracked [next delivery](plans/0036-remote-workspace-execution.md).

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
