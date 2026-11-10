# 0092 — One register file, a pinned project root, language servers that come back

- Status: **in force**
- Date: 2026-11-10
- Amends: [0074](0074-input-registers-and-search-case.md) (field registers),
  [0053](0053-unified-search-workspace.md) (the Search
  replacement key), [0049](0049-product-and-architecture-handoff.md) §4.2/§4.6
  (external files and language-service context)

Two owner reports, one cause each: state that should have one owner had two.
The same branch also adopts four items from the 0.56 critique
([0091](0091-editor-critique-0.56.md)) and fixes two flaky tests; see §3a, §3b.

1. "I yank a word and cannot paste it in the file finder, grep, search or `:`.
   Only the system clipboard works. Register copy is buggy in general."
2. "I launch in `/workspace`, go to a definition in the Rust stdlib, open the
   file finder — and the stdlib directory is now the project. LSP keeps
   disappearing after navigating back and forth."

## 1. Registers

### 1.1 Root causes (line numbers are `main` at 0.56.0)

- **The store was a bare map with one rule.** `editor/registers.rs:95`
  `set_register` inserted into `HashMap<char, Register>` and mirrored every
  write into `"`. There was no `"0`, no `"1`–`"9` shift, no `"-`, no `"A`
  append, no read-only registers; `"_` was *stored* (`"_p` pasted it back);
  a delete and a yank were the same write. `editor/macros.rs:14` kept
  recordings in a second map, so `"ap` never showed a macro and `@a` never ran
  a yanked register.
- **Four read paths.** `p` (`registers.rs:114`), insert `Ctrl-R`
  (`insert.rs:55`), the `: / ?` line (`normal/pending.rs:117`) and pickers
  (`picker/keys.rs:125`, `:159`) each called `register(name)` themselves, and
  only `p` knew `"+` is an asynchronous clipboard read. `Ctrl-R +` in a prompt
  pasted whatever strop last *wrote* to `+`.
- **Search's Find and With fields could not read a register in insert mode.**
  `picker/keys.rs:109` `Key::CtrlR if search => toggle_search_replacement()`
  took the key before the field saw it (0074 recorded this as a deliberate
  exception; it is the "grep" half of the report). The `: / ?` line refused
  `p`/`P` in its normal mode (`field.rs` routed them to "not supported in an
  input field"), and `"ap` was refused everywhere (`field.rs:198`).
- **Visual mode dropped the register.** `visual.rs:131`/`:163` wrote
  `set_register(None, …)`, and the visual walker never parsed `"x`, so `"ay`
  in visual did nothing at all.
- **What the owner actually pressed.** A plain `yiw` never reaches the system
  clipboard, and the terminal's paste (Ctrl-V in Windows Terminal) only ever
  carries the system clipboard. Nothing on screen named `Ctrl-R "`. So "only
  the system clipboard works" was true of every key the pickers advertised.

### 1.2 What established editors do

Sources fetched for this plan (Neovim `master`, Helix `master`, Zed `main`):

- **Neovim `:help registers`** (`runtime/doc/change.txt`): `""` is filled by
  every d/c/s/x/y and points at the last used register; `"0` is "the most
  recent yank command, unless the command specified another register"; `"1`
  takes the most recent delete or change "(even when the command specified
  another register), unless the text is less than one line", and always with
  `% ( ) ` / ? n N { }`; 1→2…8→9 shift; `"-` takes deletes of less than a
  line "except when the command specifies a register"; `A–Z` append; `". "%
  ":` are read-only; `"_` stores nothing.
- **Neovim source**: `src/nvim/ops.c` `op_delete` — named register first;
  then `if (motion_type == kMTLineWise || line_count > 1 || use_reg_one)`
  shift and write `"1` (even when named); then `if (regname == 0 && not
  linewise && line_count == 1)` write `"-`. `src/nvim/register.c`
  `get_yank_register` maps "no register" to `y_regs[0]`;
  `shift_delete_registers(y_append)` leaves the unnamed pointer on the named
  register when appending; `op_yank_reg`'s append branch: "kMTLineWise
  overrides kMTCharWise and kMTBlockWise", charwise onto charwise continues
  the last line.
- **`c_CTRL-R` / `i_CTRL-R`** (`cmdline.txt`, `insert.txt`): `Ctrl-R {reg}`
  inserts the register; `" % # * + / : - . =` are allowed; a multi-line
  register is inserted into the command line with `^M` between lines
  (`cmdline_paste_reg`).
- **`'clipboard'`** (`options.txt`): default `""` — the unnamed register and
  `"+` are separate. `unnamedplus` routes every unnamed yank/delete/put
  through `"+`. kickstart.nvim, LazyVim (off over SSH), AstroNvim and NvChad
  all set `unnamedplus`.
- **Zed vim mode** `use_system_clipboard`: `always` (default), `never`,
  `on_yank`. **VSCodeVim** `vim.useSystemClipboard`: default `false`.
  **Helix** (`book/src/registers.md`, `keymap.md`): `"` is the default yank
  register, `+`/`*` are the clipboard, no implicit sync; `Space y` / `Space p`
  are the separate clipboard keys — the model strop already copied.

### 1.3 Design

**One store.** `editor/registers/store.rs` `RegisterStore` holds `""`,
`"0`–`"9`, `"-`, `"a`–`"z`, the clipboard cell and `":`. It is pure (no editor,
no IO) and states Neovim's rules once: `write(name, register, Origin)` where
`Origin` is `Yank` or `Delete { reg_one }`. Callers say which they are —
`yank_register` / `delete_register` — and never touch a cell. `"_` returns
before anything moves; `". : / %` and unknown names are typed `RegisterError`s,
checked *before* an operator edits (`register_writable`), so `".dd` deletes
nothing.

**One read path.** `Editor::paste_from(name, PasteDestination)` resolves the
name through `read_register` and delivers to `Put`, `Insert`, `Prompt` or
`Picker`. `"+`/`"*` resolve to `RegisterRead::Clipboard`: the same provider job
as before, now carrying its destination, delivered through the same function
when it lands and dropped (with a message) if that document, prompt or picker
is no longer the focus. `". ": "/ "%` are computed views of editor state.

**Every text input.** The field machine (`field.rs`) replies
`ReadRegister(name)` for insert-mode `Ctrl-R {reg}` and for normal-mode
`p`/`P`/`"{reg}p`; the `: / ? |` line and every picker field route that reply
to `paste_from`. Search's replacement toggle moves to **`Ctrl-T`** so `Ctrl-R`
means one thing in every input (amends 0053 and retires 0074's exception;
`Space R` still opens Search with With visible). Fields still do not *write*
registers: a query is not a document.

**Macros are registers.** A finished `qa` writes its keys into `"a` as vim's
text (`Esc` is `\x1b`); a later write to `"a` drops the typed recording and
`@a` replays the register's text as keys.

**Visual mode takes `"x`.** The visual walker parses the selector and
`d y c x` (charwise, linewise, blockwise, occurrence cascade) use it.

**Deviations, recorded.**

- A linewise register of exactly one line pastes that line into a field (`yy`
  then `/ Ctrl-R "` searches for the line). Vim inserts a literal `^M`; a
  one-line field has no use for it. Text with inner line breaks is refused
  with a message, as before — never truncated.
- Appending to or from a blockwise register follows the type rule above but
  does not re-pad rows to a common width.
- `"#` (alternate file) and `"=` (expression) do not exist.

### 1.4 Clipboard policy: a yank is a copy (Zed's `on_yank`)

Owner decision, 2026-11-10: "the editor works as you'd expect". The reported
failure was literally *yank, then the terminal's paste does not carry it*.

- **A plain yank also writes the system clipboard.** `y` with no register (or
  `"0` by name) stages the same OSC52 payload `"+y` does and fills the `"+`
  cell. It still fills `""` and `"0` exactly as before.
- **Deletes and changes do not.** `dd`, `x`, `cw` and friends fill `"1`–`"9`
  or `"-` and leave the clipboard alone. Neither does a yank into a named
  register (`"ayw`): naming a register is saying where the text goes.
- **`p` reads the unnamed register, not the clipboard.** `"+p`, `Space p` and
  `Ctrl-R +` read the clipboard. The terminal's own paste always does.

Why this and not the two Neovim options:

- *Vim's default* (`clipboard=`) keeps yanks inside the editor. It is what
  `nvim -u NONE` does, and it is what failed the owner: kickstart, LazyVim,
  AstroNvim and NvChad all turn it off, so it is not what a Neovim user
  expects to happen either.
- *`unnamedplus`* makes every `p` a clipboard **read** — a provider
  subprocess. strop's reads are jobs (doctrine 3), so `p` would stop being
  synchronous, and would fail where no provider is installed (stock WSL has
  none of `wl-paste`/`xclip`/`xsel`). It also sends every `dd` and `x` to the
  clipboard, the standing complaint about that option and the reason Zed has
  `on_yank` at all.
- *`on_yank`* needs only the write path, which is OSC52 and works in WSL, over
  SSH and in tmux; `p` stays a synchronous read of strop's own store; a delete
  never destroys what was copied in another application.

Register contents are identical to Vim's default — the differential harness
still diffs `"" "0 "1 "2 "- "a "b` against `nvim -u NONE` — so the deviation is
exactly one side effect of `y`.

Two limits, both deliberate:

- **Size.** A plain yank over 1 MiB is not mirrored (`CLIPBOARD_MIRROR_MAX`);
  the message says so and names `"+y`, which is never capped. OSC52 is base64
  through the terminal, and `ggyG` on a large file must not stall it.
- **Traces.** The OSC52 write is a `terminal.osc52` replay request carrying
  the text. Requests are recorded only under the content opt-in
  (`--log-content`; `Tape::captures`), where keys and document text already
  are; a metadata trace records nothing new. No redaction was added. A full
  trace recorded before this change that contains a plain yank no longer
  replays (the request is new); none is checked in.


## 2. The project root and language servers

### 2.1 Root causes

- **The scope followed the focused file.** `editor/workspaces.rs:196`
  `current_workspace_root()` took `directory_context()` — the focused file's
  *parent directory* (`directory/mod.rs:218`) — and used it as the scope
  whenever no selected root covered it. Inside the project the launch root
  covers every file, so nothing showed; one `gd` into
  `~/.rustup/toolchains/…/library/core/src/option.rs` and Files, Search and
  new terminals were scoped to `library/core/src`. `Editor::new`'s own comment
  says "a file's own dir is not the project"; this function contradicted it.
- **A library file had a server only by inheritance.** `lsp_server_for`
  (`lsp/lifecycle/mod.rs:136`) resolves binding → carried jump context →
  attachment whose root covers the path. A stdlib file is covered by no root,
  so it is served only while the `gd` that opened it left a context behind
  (0049 §4). Reach the same file from the drifted file finder, `:e`, the
  jumplist after a close, or a restored session and there is no context:
  discovery runs for it (`lsp/attach.rs:289`–`294` →
  `strop-lsp/src/registry.rs:282`, "the git root, else the file's directory")
  and roots a second rust-analyzer in the library directory. That is the
  server "disappearing": the file is bound to a server that knows nothing of
  the project.
- **A crash dropped everything and restarted nothing.** `lsp/routing.rs:29`
  → `lsp_failed` (`lsp/state.rs:525`) removes the server, every binding and
  every jump context. Nothing re-attached until the user switched buffers or
  repeated a request, and the retry (`state.rs:370`, "out-of-context external
  files stay isolated") was open only to files *inside* the failed root. A
  library file could never get its server back; the next attach for it took
  the path above.

### 2.2 What established editors do

- **Neovim** (`runtime/doc/editing.txt`): one global working directory,
  changed only by `:cd` (`:tcd`/`:lcd` scope it to a tab or window);
  `'autochdir'` is off by default; `:cd` "does not change the meaning of an
  already opened file". `vim.lsp.start` "reuses an already running client if
  one is found matching `name` and `root_dir`" (`runtime/doc/lsp.txt`).
  nvim-lspconfig's `lsp/rust_analyzer.lua` `is_library(fname)`: for paths under
  `$RUSTUP_HOME/toolchains`, `$CARGO_HOME/registry/src`,
  `$CARGO_HOME/git/checkouts` it returns the existing client's `root_dir`, so
  the library file attaches to the running client instead of starting one. On
  exit (`vim/lsp/client.lua` `_on_exit`) Neovim notifies and does not restart;
  `:lsp restart` is manual.
- **rust-analyzer** (`project-model/src/workspace.rs` `to_roots`,
  `base-db/src/input.rs` `SourceRoot.is_library`): sysroot and registry
  packages are library source roots of the workspace that depends on them;
  `handle_did_open_text_document` accepts any path, with no workspace
  membership check. One server, library files included.
- **Helix** (`book/src/languages.md`, `helix-loader` `find_workspace`): "The
  workspace root is found once by walking upward from the current working
  directory"; it is the ceiling for LSP roots and the file picker's root.
  The current file's directory is a separate, explicit picker.
- **VS Code**: the workspace folder is the opened folder. rust-analyzer's
  extension registers `documentSelector: [{ scheme: "file", language: "rust"
  }]` — no folder pattern — so any Rust file on disk goes to the one running
  client (vscode-languageclient `DidOpenTextDocumentFeature`). The default
  error handler restarts a closed server until it has crashed 5 times in 3
  minutes (`client.ts` `DefaultErrorHandler`, `maxRestartCount ?? 4`).
- **Zed** (`crates/project/src/lsp_store.rs` `open_local_buffer_via_lsp`): a
  definition outside every worktree creates an *invisible* worktree and
  registers the originating server for it
  (`register_language_server_for_invisible_worktree`); the project panel and
  the project itself are unchanged.

Every one of them pins the project and sends library files to the server that
already owns the project.

### 2.3 Design

**The project root is `Editor::cwd`, fixed at launch.** `current_workspace_root`
returns it for any local file no selected root covers. Only three things scope
elsewhere, all explicit: a Directory buffer the user browsed to, a terminal's
own launch location, and an unbound remote resource. `:pwd` shows the root.
Each consumer, after this plan:

| Consumer | Root |
|---|---|
| Files, Search (`picker/mod.rs`, `picker/search.rs`) | `current_workspace_root()` — pinned |
| `Space t` / `:terminal` (`terminal/lifecycle.rs`) | `current_workspace_root()` — pinned |
| `:e relative` (`directory/mod.rs` `open_context`) | `cwd` — already pinned |
| Git (`git/context.rs` `discover_git`) | the repository of the focused file — per document by design (blame, hunks and diffs are the file's); unchanged. A library file has none, and returning to a project file restores the project's |
| LSP root (`lsp/attach.rs`) | config layer root, else the file's git root, else its directory — unchanged for files inside the project |
| LSP owner of a file outside the project | the project's server (below) |

**A local file outside the project belongs to the project's server.**
`lsp_server_for` gains a last step, `lsp_project_server`: for a local path
outside `cwd`, the attachment for its language (C and C++ are one family)
whose root is inside or covers `cwd` — when there is exactly one. The lookup
is in-memory and deterministic; didOpen then records an ordinary binding.
Discovery is therefore never started for a library file while the project has
a server for its language.

This amends 0049 §4.6. That rule — a manually opened external file gets an
explanation, "not arbitrary reuse" — stands when the choice would be
arbitrary: with two project servers for the language, or none, the file keeps
the old behaviour and the old message. 0049 §4.2's "never scan unrelated live
servers" is kept: a server rooted outside the pinned project is never an
owner. 0049 §4.7's trust concern is kept: nothing is discovered or executed
from the library's directory.

A file from a *different* project opened here goes to this project's server
(the VS Code rule). Working on that project is `strop` launched there.

**A crashed server restarts, visibly, a bounded number of times.** On
`LspEvent::Failed` for a local server that had reached Ready,
`lsp_restart_plan` captures — before teardown — the documents it served
(bindings and carried contexts, library files included) and a representative
inside its root. `lsp_restart` spends one of `RESTART_BUDGET = 3` restarts for
that `(language, root, target)`, re-runs discovery for the representative and
reports `lsp: NAME failed — restarting (1/3) — :explain for details`; the
`LSP!` badge and `:explain` keep the reason until the replacement is Ready.
When the replacement is placed, `lsp_reopen_restarted` opens every captured
document on it. Past the budget the message says `not restarted; repeat a
request to retry`; an explicit request refills the budget and, from a library
file, leads with an open project file so discovery finds the same workspace.

The budget is a count, not vscode-languageclient's "5 times in 3 minutes":
the restart decision gates a recorded `lsp.attach` request, and a decision
that reads the wall clock cannot be replayed from a trace. A server that never
became Ready is a start failure, not a crash — restarting it would fail the
same way, so its refusal stands (unchanged). Remote servers are not
auto-restarted: their process is an `ssh` client and the connection has its
own recovery.

### 2.4 Left for a later plan

- **`:cd`.** The root is changed "only by an explicit command", and today no
  such command exists. Adding it means re-anchoring everything keyed by
  `cwd`: relative buffer paths, the session and recovery-checkpoint files, the
  git repository, picker caches. It deserves its own plan; `:pwd` ships now.
- **An outside file opened before any project file of its language** (a
  session restored onto a stdlib file) still discovers its own server: there
  is no existing client to own it. nvim-lspconfig has the same rule.

## 3. Left for the keys track

Found by the new register cases in the differential harness, or adjacent to
this work. None is a register or root bug; each is stated so it can be picked
up as written. Repros are `text` / `keys`, results are final text and the
unnamed register, Neovim 0.9.5 `-u NONE`.

1. **`dw` on the last word of a line joins the next line.**
   `"alpha beta\ngamma\n"` / `wdw`. strop: `"alpha gamma\n"`, register
   `"beta\n"` (and it lands in `"1`, not `"-`). Neovim: `"alpha \ngamma\n"`,
   register `"beta"`. Rule: `:help w`, "Another special case" — with an
   operator, when the last word moved over ends its line, the operated text
   ends at the end of that word. Site: `strop-grammar/src/resolve/mod.rs`,
   the `Motion::WordForward | Motion::BigWordForward` arm (the range is
   `[cursor, word_forward(…))` with no end-of-line stop). `cw` has its own
   arm and is unaffected; check `dW`, `yw` and counts (`d2w` across a line
   break deletes through the first word of the next line in Vim).
2. **An exclusive motion ending in column 1 takes the line break.**
   `"a b\n\nc\n"` / `wd}`. strop: `"a \nc\n"`, register `"b\n"`. Neovim:
   `"a \n\nc\n"`, register `"b"`, in both `"1` and `"-`. Rule: `:help
   exclusive-linewise` — if the motion is exclusive and ends in column 1, the
   end moves to the end of the previous line and the motion becomes
   inclusive; if in addition the start is at or before the first non-blank,
   it becomes linewise. Applies to every exclusive motion (`}` `{` `w` `b`
   `/pat` `` ` ``), so it belongs in one place after motion resolution, with
   the `w` rule (1) applied first.
3. **`d}` in the last paragraph deletes backwards.**
   `"a b c\n"` / `wd}`. strop: `"b c\n"`, register `"a "` (the range runs from
   the line start to the cursor). Neovim: `"a \n"`, register `"b c"`. With no
   blank line below, `}` goes to the end of the last line; strop's
   `ParagraphForward` resolves to an offset before the cursor here. Site:
   `strop-grammar/src/resolve/motions.rs`.
4. **Visual `p` / `P` do not exist.** `yiwwviwp` reports "not an editor
   command: p" (0091 B13). Vim replaces the selection with the register and
   puts the replaced text in `""` (`P` leaves `""` alone). The register side
   is ready: `delete_register` for the replaced text, `paste_from(name,
   Put { .. })` for the put, and the visual walker already parses `"x`.
5. **`:cd`.** The project root is pinned (§2.3) and nothing changes it. A
   `:cd` must re-anchor everything keyed by `Editor::cwd`: relative buffer
   paths (`self.cwd.join(path)` is how every local path is made absolute),
   the session and recovery-checkpoint files, picker source caches, and the
   LSP "outside the project" test in `lsp_project_server`. `:pwd` ships.
6. **Fields do not write registers.** `yw` / `dw` in a picker or prompt field
   resolve and store nothing (`field.rs`, `Some(Op::Yank) => {}`).
7. **`"#` and `"=`** (alternate file, expression) do not exist; appending to
   a blockwise register does not re-pad rows.

One divergence the cases found was a one-line fix and ships here: `dd` on the
last line left the cursor on the phantom row past the final line break
(`normal/execute.rs`; Neovim lands on the new last line).

## 3a. Adopted from the 0.56 critique (0091)

- **B7, `Ctrl-W`.** The prefix had no rows, so the walker rejected it and the
  next key ran in Normal (`Ctrl-W v` started Visual). `keymap/bindings/panes.rs`
  now binds `h j k l w W p s v n c q o =` to `Editor::window_command`, on the
  existing flat pane layout: directions wrap as `Space w` does; `p` returns
  to the pane that last had focus (`Pane::focused`, a view-local stamp); `n`
  splits below onto an empty buffer; `c` refuses on the last window, `q` is
  `:q`; `o` drops the other panes (buffers stay open); `=` is bound and does
  nothing because panes are always equal. `Ctrl-W Ctrl-{key}` is the same
  command however the terminal reports the second key — its own code, a raw
  control character, or Backspace/Enter for Ctrl-H/Ctrl-J (`window_key` in
  `input.rs`); `Ctrl-W Ctrl-C` cancels. Any other key yields
  `Action::Unbound` — "ctrl-w x is not bound" — and is consumed: the walker
  never hands a `ctrl-w` path to the grammar. `Space w` is unchanged.
- **B10, the welcome card.** Its last line is now `space f finds a file ·
  space / searches the project`. The card also takes the width its lines need
  when the terminal has it: at 80 columns the 60 % rule had been clipping
  `:w save` and the last line.
- **README.** The header is "A modal text editor with Vim's keys"; the coined
  lineage line, the logo alt text describing a frame the binary cannot draw,
  and "the preview cannot lie" are gone. The table of what is shown before a
  change stays, in plain words.
- **0001 §2.1** carries a "superseded" note pointing at 0091 §3.1.

The tagline itself still appears on the welcome card, in `--version`,
`install.sh` and the crate description. 0091 §3.3 recommends retiring it as
the headline once the keys release lands; that is a positioning decision for
that release, not made here.

## 3b. Two flaky tests

- **`full_trace_roundtrips_literal_keys_and_closes_cleanly`** compared the
  whole `state` object of a live run with its replay. `completion.publication`
  in that object is the completion mailbox's memory accounting (bytes charged,
  high-water mark, deliveries destroyed), which depends on when the worker
  thread ran. `tests/tracing.rs` `replayed_state` now leaves that one object
  out of both comparisons that had it; everything a user can observe is still
  compared.
- **`kitty_shifted_punctuation_opens_ex_commands`** failed on main CI at
  `assert!(tui.wait_exit().success())` with nothing to read: strop's stderr
  is the PTY. `Tui::expect_clean_exit` drains the PTY after the child exits
  and, on a non-zero status, prints the status, the screen left behind (the
  main screen, where the error is), the last terminal bytes and the trace
  tail. Every `wait_exit().success()` site uses it. The cause is not known;
  the next failure will name it.


## 4. Tests

Differential, against pinned Neovim (`crates/strop/tests/differential.rs`):
the harness now diffs `"0 "1 "2 "- "a "b` as well as `""`, over 23 new cases —
yank fills `"0`; the numbered shift; small deletes; `x`; a named small delete
skipping `"-`; a named line delete still filling `"1`; `d%` and `d/pat`
within a line filling `"1` *and* `"-`; `"A` append, charwise and linewise;
`"_`; `"0P` and `"2p`; `cw`; visual `"by` and `V"ad`; insert `Ctrl-R "` and
`Ctrl-R a`; `/ Ctrl-R "`; `".p`, `"/P`, `":p`; `dd` on the last line. The
existing macro case now also requires `"a` to hold the recording.

State (`strop-engine`):

- `registers/store.rs` — seven unit tests of the pure rules.
- `picker/field_register_tests.rs` — `yiw` then `Ctrl-R "` in Files, Search,
  `/` and `:`; Search's With field and `Ctrl-T`; named, numbered and
  small-delete registers read identically in a field and by the store;
  `": "/ "%` and the refused `":dd`; one-line linewise paste and multi-line
  refusal; field normal-mode `p`, `P`, `"ap` in a picker and on the `:` line;
  a clipboard read landing in the prompt or picker that asked, and nowhere
  once it closed.
- `tests/register_tests.rs` — a recording is register content and a yanked
  register replays as keys; the black hole.
- `workspaces.rs` — the reported drift: focus a file outside the project and
  Files, Search and `:pwd` still name the launch root; a browsed directory is
  the explicit exception.
- `lsp/tests/project.rs` — a library file with no navigation context resolves
  to the project's one server, not to another language's or to a server
  rooted elsewhere; a crash restarts within its budget, carries the library
  file, keeps the failure visible, stops at the budget, and never restarts a
  server that did not start.
- `lsp/tests/navigation.rs` — the real-wire fixture (a scripted server that
  exits on a request from the library file) now requires the restart: one
  replacement, both documents bound to it, the library file re-opened on the
  wire, no second restart from navigating back and forth. The 0049 §4.6
  message test now sets up the ambiguous case it still covers.

Golden frames: the Files and Search hint rows (`ctrl-r "  paste yank`,
`ctrl-t  replace`).

Added with §1.4, §3a and §3b:

- `registers/store.rs` and `tests/register_tests.rs` — a plain yank is
  published and fills `"+`; deletes, `x` and named yanks are not; the 1 MiB
  limit keeps the yank and says so.
- `editor/panes.rs` — for every printable key after `Ctrl-W`: a bound key
  dispatches, any other reports "ctrl-w {key} is not bound" and leaves panes,
  focus, cursor, text and mode exactly as they were (0091 §7's acceptance
  test); and the commands drive the pane machinery, including held-Ctrl forms
  and `Ctrl-C`.
- Golden: `welcome-{hone,vellum}-{140x40,80x24}`.
