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
| Project replace, a rename across files, file operations | Every change as a diff. `Enter` applies and writes it; `u` in the review puts the buffers and the files back. |
| Macros, shell commands | Nothing. |

## Install

```sh
curl -fsSL https://strop.dev/install.sh | sh     # prebuilt static Linux/WSL binary
```

Already installed? `strop update` updates it in place. It downloads and
checks the new version itself; curl and tar are not needed.

strop runs on Linux and WSL, x86_64. The older crates.io, Homebrew, Linux
aarch64 and macOS releases can still be downloaded but get no new versions.
On Windows, run strop inside WSL.

## The keys on one card

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
Space g  g status · c commit · b branches · l log · h history · B blame · s/u/X/p hunk
```

`Space ?` lists every key in the editor, and `:help TOPIC` jumps to a
section of that list.

## The docs

- **The command line**: `strop file.rs`, `strop remote ssh://user@host:2222/path`,
  the other commands and the options → **[docs/cli.md](docs/cli.md)**
- **Search and filters**: the file finder, project search and replace,
  and the filters (`language:cpp path:tests`, `AND`/`OR`/`NOT`) that work
  in every search field → **[docs/search.md](docs/search.md)**
- **Editing**: Vim's keys, the `:` line, registers, windows, indentation,
  themes, completion, language servers, sessions →
  **[docs/editor.md](docs/editor.md)**
- **Git**: status with staging by file, hunk or section, commit with the
  message in a buffer, branch switching, log, history and blame →
  **[docs/git.md](docs/git.md)**
- **SSH and remote editing**: uses your own `ssh`, nothing installed on
  the host. Once connected you can edit and save; a save is refused if the
  file changed on the host; search, Git, language servers and terminals run
  there → **[docs/remote.md](docs/remote.md)**
- **Files and directories**: browse a directory, and create, rename, move,
  copy and delete files, on your machine and over SSH. Every change is
  shown for review before it happens →
  **[docs/filesystem.md](docs/filesystem.md)**
- **Terminals**: `Space t` opens a shell in a split below; `Ctrl-\ Ctrl-N`
  leaves it, and its output is a buffer you can search and yank from →
  **[docs/terminal.md](docs/terminal.md)**
- **Scripts and bug reports**: run strop from a script of keys with no
  screen, and record a session to attach to a bug report →
  **[docs/scripting.md](docs/scripting.md)**
- **Vim compatibility**: every key strop has, and the Neovim keys it does
  not → **[docs/vim-compat.md](docs/vim-compat.md)**

## What you can rely on

- **Typing never waits.** Search, git and language servers work in the
  background; a slow one never holds up a key.
- **Everything is a buffer.** Terminals, directory listings, git views and
  search results all take the usual motions, search and yank. A collection
  of search hits is one you edit: the changes land in the files.
- **One binary, nothing to set up.** No plugins, no downloads when it
  runs; languages and themes are built in.
- **Vim's keys, checked.** Where strop differs from Neovim it is a bug,
  and a test suite that runs both side by side finds it.

The design notes are in [plans/](plans/).

## Reporting a bug

```sh
strop --log=issue.jsonl path/to/file.rs
strop replay --script issue.jsonl > replay.keys   # the keys, as a script you can read
strop --log=issue-full.jsonl --log-content path/to/file.rs
strop replay issue-full.jsonl                     # show the whole session again
```

What a log contains and how to replay it:
[docs/scripting.md](docs/scripting.md).
**Read a log before you share it.** It contains your keys, commands and
paths.

## How it is checked

The keys are checked against Neovim itself: the same key sequences run in
both editors and the results are compared. The code that moves cursors,
marks and selections through an edit is a small separate crate
(`crates/strop-kernel`) that is translated to Lean 4 and proved correct
there. The rest is covered by tests that drive the real binary, including
over a real SSH server. The details are in
[plans/0086](plans/0086-verified-foundation.md), and
`docker compose run --build --rm proofs` prints the list of what is proved.

## Security

Report vulnerabilities privately; the policy and privacy statement live in
[SECURITY.md](SECURITY.md).

## Links

- [strop.dev](https://strop.dev) — demo, palettes, install
- [docs/](docs/) — the documentation
- [plans/](plans/) — design notes

MIT license · © 2026 [Tarek Nawara](https://github.com/tknawara)
