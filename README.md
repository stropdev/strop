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

## Reporting a bug

```sh
strop --log-file issue.jsonl path/to/file.rs
# Or capture file/paste text and cell grids for a self-contained input reproducer:
strop --log-file issue-full.jsonl --log-content path/to/file.rs
strop --headless steps.keys path/to/file.rs --log-file headless.jsonl
strop --replay-script issue-full.jsonl > replay.keys
```

Attach the JSONL file with the observed and expected behavior. Logs contain keys,
commands, paths and diagnostic messages; full-content logs also contain documents,
pastes and rendered cells. **Inspect before sharing.** Existing files are never
overwritten. The extracted script replays inputs from a scratch snapshot, not
external service results or filesystem state; inspect it before executing it.

See [tracing design](plans/0029-session-tracing.md), the
[prioritized review/roadmap](plans/0028-roadmap-and-review.md), and the
[hardening proposal (not implemented)](plans/0030-correctness-hardening.md).

## Links

- [strop.dev](https://strop.dev) — demo, palettes, install
- [plans/](plans/) — the numbered design contracts everything answers to

MIT license · © 2026 [Tarek Nawara](https://github.com/tknawara)
