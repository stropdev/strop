# 0091 — One register file, a pinned project root, language servers that come back

- Status: **in force**
- Date: 2026-11-10
- Amends: [0074](0074-input-registers-and-search-case.md) (field registers),
  [0053](0053-unified-search-workspace.md) (the Search
  replacement key), [0049](0049-product-and-architecture-handoff.md) §4.2/§4.6
  (external files and language-service context)

Two owner reports, one cause each: state that should have one owner had two.

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

### 1.4 Clipboard policy: Vim's default, not `unnamedplus`

`""` and `"+` stay separate. `y`/`d`/`p` never touch the system clipboard;
`Space y`, `"+y`, `Space p`, `"+p` and `Ctrl-R +` do.

Why, given that the popular Neovim distributions all turn `unnamedplus` on:

1. **It is what `nvim -u NONE` does**, and that is the oracle the differential
   harness diffs against (0001 §5.10). A zero-config editor's defaults are
   Vim's defaults.
2. **The async invariant (doctrine 3).** `unnamedplus` makes every `p` a
   clipboard *read* — a provider subprocess. strop's reads are jobs; `p` would
   stop being synchronous and deterministic, and would fail outright where no
   provider is installed (stock WSL has none of `wl-paste`/`xclip`/`xsel`).
3. **Deletes would clobber the clipboard** (`dd`, `x`, `cw`) — the standing
   complaint about `unnamedplus`, and the reason Zed grew `on_yank`.
4. **Traces.** Every OSC52 write is a recorded `terminal.osc52` request
   carrying the text. Mirroring each yank would put yanked content in every
   trace and invalidate recorded tapes.

The owner's actual need — paste a yank into a picker — is met by the register
keys working everywhere and being *named* on the picker: Files and the other
list pickers show `ctrl-r "  paste yank` in their hint row, and `?` lists it.
`on_yank` mirroring (Zed's middle option) is the smallest change if this
decision is revisited: one `osc52` assignment in `write_register`.

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

## 3. Found on the way, not fixed here

The new register cases in the differential harness turned up grammar
divergences that are not register bugs. One was a one-line fix and ships:
`dd` on the last line left the cursor on the phantom row past the final line
break (`normal/execute.rs`; Neovim lands on the new last line). Three are
resolver changes that touch every operator and want their own change:

- `dw` on a line's last word deletes through the line break and joins the
  next line. Vim stops at the end of the word (`:help w`, "Another special
  case").
- An exclusive motion that ends in column 1 of a later line (`d}` from
  mid-line) takes the line break. Vim moves the end back to the previous
  line's end, or makes the motion linewise (`:help exclusive-linewise`).
- `d}` in the last paragraph of a buffer deletes backwards to the line start
  instead of forwards to the buffer end.

Visual-mode `p`/`P` (replace the selection with a register) does not exist.

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
