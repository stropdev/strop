<div align="center">
  <img src="logo.svg" width="480" alt="strop">
  <h3>A modal text editor with Vim's keys.</h3>
  <p>One static binary. File finder, project search and replace, git and language servers are built in.</p>
  <p>
    <a href="https://strop.dev">strop.dev</a> ·
    <a href="https://github.com/stropdev/strop/releases">what's new</a>
  </p>
</div>

![strop demo](demos/demo.gif)

**strop** is a modal text editor written in Rust. Its keys are Vim's, checked
against Neovim key for key by a test suite of about four thousand key
sequences; [docs/vim-compat.md](docs/vim-compat.md) lists what is there and,
at the end, what is still missing. There are no plugins to install: finding files, searching
and replacing across a project, git blame and history, and language servers
come with the binary.

## What you see before a change happens

Most Vim commands run the moment you finish typing them, here as in Vim.
A few longer ones show what they will change first:

| Command | What you see before it runs |
| --- | --- |
| `dw`, `ci[`, `dfx`, `.` | Nothing. They run at the last key, as in Vim. |
| `vi[` then `c`, and other Visual selections | The selection, before you press the operator. |
| `d/pattern` while you type the pattern | The range that will be deleted, until Enter. |
| `:s` while you type it | The changed lines, before and after, and how many. |
| Project replace, file operations | Every change as a diff. You apply it, then save, as separate steps. |
| Macros, shell commands | Nothing. |

## Install

```sh
curl -fsSL https://strop.dev/install.sh | sh     # prebuilt static Linux/WSL binary
```

Already installed? `strop update` self-updates tarball installs. Downloads,
SHA-256 verification and archive extraction run entirely in Rust; curl and
tar are not required.

Supported target: Linux/WSL x86_64 (`x86_64-unknown-linux-musl`). Release
0.47.2 narrows current prebuilt distribution to this target; historical
crates.io, Homebrew, Linux aarch64 and macOS releases remain available but
receive no new versions. Windows Terminal users run strop inside WSL.

## The grammar in one card

```
h j k l w b e 0 ^ _ $ gg G %  motions          i a A o O            insert
d y c > < + motion/object     operators        dd yy cc D C Y s x   shortcuts
iw i" i' i( i[ i{             text objects     f t / ? n N          find & search
v V                           visual           u ctrl-r .           undo, redo, repeat
"a … "+                       registers        :w :q :e :help        ex line
Q                            multicursor      Space c              cursor below

Space  f files · o remote · b buffers · / Search · R replace · ? help
Space w  - horizontal · v vertical · 1-9 focus · h/j/k/l/w move · q close
Ctrl-W   h j k l w W p move · s v n split · c q o close · = equal
Space g  l log · h history · b blame gutter · y/o permalink · u/s/p hunk
```

`Space ?` lists every binding in the editor; `:help` explains each surface.

## The tour

- **The CLI** — `strop file.rs`, `strop remote ssh://user@host:2222/path`,
  `strop replay trace.jsonl`: one binary, subcommands for the occasional
  jobs, `strop help` for the whole map → **[docs/cli.md](docs/cli.md)**
- **Search and the query language** — one qualifier grammar
  (`language:cpp path:tests`, Boolean `AND`/`OR`/`NOT`) shared by file
  find, search/replace, symbols and the jump list →
  **[docs/search.md](docs/search.md)**
- **Editor surfaces** — themes, completion, structural + label-colon
  indentation, syntax, sessions → **[docs/editor.md](docs/editor.md)**
- **SSH and remote editing** — plain OpenSSH, nothing installed on the
  host: a connected host is editable, saves refuse a file changed on the
  host, and search, Git, language servers and terminals run there; for
  containers, `:terminal` then `docker exec -it <container> sh` →
  **[docs/remote.md](docs/remote.md)**
- **Filesystem workspace** — directory browsing, locally and over SSH, and
  Review → Apply → Receipt mutations on the local filesystem →
  **[docs/filesystem.md](docs/filesystem.md)**
- **Terminals, headless scripts, traces** — `Space t` opens a real PTY in
  a split below, Neovim's `Ctrl-\ Ctrl-N` and a live NORMAL mode; the
  scripted driver, and how to capture a reproducible bug →
  **[docs/terminal-and-scripting.md](docs/terminal-and-scripting.md)**

Also in `docs/`: [vim-compat.md](docs/vim-compat.md) (the differential
contract against nvim) and [nucleo-decision.md](docs/nucleo-decision.md).

## The house rules

- **Async invariant**: input→render never crosses an `await`. Data sources
  (rg, git, LSP) are jobs posting onto the event loop — no exceptions.
- **Everything is a real buffer**: terminals, directory listings, search
  collections — every widget still knows how to be a text editor.
- **Zero-config single static binary**: no required setup, no downloads at
  runtime; grammars, queries and palettes are compiled in. rustls only —
  a dependency dragging in openssl is a bug.
- **Fidelity**: the vim grammar is the product; deviations are bugs, found
  by the differential harness against nvim, not by vibes.

Design contracts live in [plans/](plans/) — numbered, load-bearing, and
the authority for every behavior above.

## Reporting a bug

```sh
strop --log-file issue.jsonl path/to/file.rs
strop replay --script issue.jsonl > replay.keys   # the keys, inspectable
strop --log-file issue-full.jsonl --log-content path/to/file.rs
strop replay issue-full.jsonl                     # full forensic replay
```

Details, content-privacy knobs and the input-only script extraction:
[docs/terminal-and-scripting.md](docs/terminal-and-scripting.md).
**Inspect logs before sharing** — they contain keys, commands and paths.

## How it is checked

The vim grammar is checked against Neovim itself by a differential harness.
At the centre sits a small, dependency-free kernel (`crates/strop-kernel`) that
Charon/Aeneas translate to Lean 4. It is proved there: how cursors, marks and
selections move through an edit, which edit batches are safe to apply, and how
slot generations retire. Calibration mutants show each theorem actually
constrains the code. Everything further out is covered by tests that drive the
real binary over real sshd. The design is
[plans/0086](plans/0086-verified-foundation.md); the claim list is what
`docker compose run --build --rm proofs` prints.

## Security

Report vulnerabilities privately; the policy and privacy statement live in
[SECURITY.md](SECURITY.md).

## Links

- [strop.dev](https://strop.dev) — demo, palettes, install
- [plans/](plans/) — the numbered design contracts everything answers to
- [docs/](docs/) — the user-facing reference

MIT license · © 2026 [Tarek Nawara](https://github.com/tknawara)
