# 0102 — The folder view: one editable buffer of names

- Status: **in force** (2026-11-13).
- Source: the owner ("the folder view — what you get from `strop .` — needs more
  polish; redesign it as you see fit and consolidate the actions"). A design pass
  drove 0.63.0 in tmux at 100x30 on this repository and on a scratch folder.
- Follows 0.64.0 (plans 0100, 0101). Finishes what 0089 (item 2) and 0091 (§5.1) asked for and never landed:
  collapse to oil's model; drop receipts, verify and undo-of-operations.

## 1. What it was (0.63.0)

```
   1 /home/tarek/workspace/strop — 28 / 28 entries
   2 ../  parent directory   ·   Enter open   Space a actions
▌  3 .claude/                     4.0 KiB  drwxr-xr-x
  21 CHANGELOG.md               189.7 KiB  -rw-r--r--
  NORMAL    main  /home/tarek/workspace/strop/ [RO]                    10%   3:1
```

A read-only listing. Changing anything went through `Space a` (18 rows) or
`:fs <verb>` (20 verbs), then a full-screen review buffer, then a full-screen
receipt buffer that stayed in the buffer list. An oil-style editable mode
(`:fs edit`, "filename draft") existed and worked, but was hidden behind the
action list with a different look.

Defects found on the binary:

1. Every file operation left a `.strop-lock-<sha256>` file beside its target,
   and the folder view listed them. Only removing a folder pruned them.
2. `Ctrl-W v/s/o/=` answered "readonly buffer" in any read-only view: the
   read-only allow-list knew `pane-*` ids but not `window-*`.
3. The cursor landed on `../` after an apply, a `:filter`, and clearing a filter.
4. Filter and dotfile state were invisible (`directory_visibility_summary` had
   no caller); the `N / M entries` count was cut off on long paths.
5. `Enter`, `-` and Backspace were documented only in a hand-written "Remote"
   block of `?`, not as binding rows.

Rough edges: absolute paths in header and modeline; the name column padded to
the longest name, so metadata jumped between folders; folder sizes (`4.0 KiB`)
as noise; permissions shown, modified time not; buffer line N was entry N−2;
`dd`/`cw`/`o`/`p` answered "readonly buffer"; `yy` yanked size and
permissions; marks duplicated Visual mode; wording like "5 exact steps; 0
refused", "descriptor-pinned parents", "checked recovery".

## 2. The design

**The folder view is one editable buffer whose text is the entry names, one per
line. Edit it with vim, `:w` shows the changes, `y` applies them.** This is
oil.nvim's model; strop already had the hard part (the draft keeps each row's
file identity through edits, yanks and undo). It becomes the only mode.

### 2.1 Buffer and look

- Text: names only, folders with a trailing `/`, folders first. No header row,
  no `../` row: line N is entry N, so `gg`, `5G`, counts and `dd` mean what they
  say.
- Size, age and git mark are drawn after the name by the renderer; they are not
  buffer text. `yy` yanks a name.
- The metadata column starts at `clamp(longest name + 2, 26, 50)`, computed once
  per listing, so it does not move while scrolling. A longer name is shown in
  full and its row drops the metadata.
- Columns: size (`96`, `8.5K`, `190K`, `2.8M`; blank for folders), age (`5h`,
  `3d`, `2w`, `1y`, against the editor clock so tests are deterministic).
  Permissions go.
- The path lives in the modeline and pane title, relative to the project root
  like a file (`strop/` for the root, `crates/strop-engine/`; absolute outside
  the root). `[RO]` goes; the usual modified mark appears while edits are
  pending. A muted summary takes the indent readout's slot: `27 items`,
  `3 of 9 · filter rs`, `22 items · dotfiles hidden`.
- One folder per buffer, as in oil, dirvish and netrw. No tree expansion.

### 2.2 The action set

| Key | Does |
|---|---|
| `Enter` | open the file / enter the folder |
| `-` | parent folder, cursor on the folder you left |
| `g.` | show or hide dotfiles (remembered per folder) |
| `:e` | list again (with pending edits it asks for `:e!`, as vim does) |
| `:filter text` / `:filter` | narrow / clear |
| any vim edit | `cw` rename, `dd` delete, `o name` create (`name/` a folder, `a/b.txt` creates `a/`), `yy` `p` copy; `cc sub/name` moves into a folder |
| `:w` | show the changes; `y`/`Enter` applies, `q`/`n` cancels |
| `u`, `:e!` | undo an edit / drop all pending edits and reload |
| `Space e` | from a file: its folder, cursor on the file |
| `Space a` | Search in this folder, Terminal in this folder, Copy path |
| `Ctrl-W v`, `Enter` | open in a split |

Removed: `:fs` and every verb, `:browse` (use `:e dir`, `:e ssh://host/dir/`),
Backspace, the `../` row, marks, receipts, operation history/verify/undo, the
copy-source and delete-disposition policies, opening a file after creating it,
and 15 of the 18 `Space a` rows. `q` records a macro, as in every editable
buffer.

Fixed rules replacing the policies: a copy copies the file on disk and is
refused ("main.rs has unsaved changes; write it first") when the source buffer
is dirty. A local delete goes to the Trash. A remote delete is permanent and the
confirm buffer says `delete`.

### 2.3 Pending changes, and the preview cannot lie

- While editing, each changed row says what it is in the metadata column:
  `was main.rs`, `new`, `from ../docs/notes.txt`, `copy of lib.rs`. One engine
  function over the draft's row origin produces it and the compiler classifies
  rows with the same function; the renderer never diffs.
- `:w` compiles, prepares and proposes. The confirm buffer renders that prepared
  batch and `y` executes that same batch.
- On full success the confirm buffer closes, the listing reloads, the cursor
  stays on its row, and the echo says `7 changes applied`.
- On partial failure the confirm buffer stays and each row gains `done` or
  `failed: <reason>` under `4 of 7 applied`; `q` returns to the listing,
  reloaded from disk.
- Leaving a folder with pending edits is allowed; it is an unsaved buffer like
  any other.
- Above 512 visible entries the buffer stays read-only and an edit answers
  "more than 512 entries; :filter first to edit names".

### 2.4 Target mock-ups (100 columns)

```
▌  1 .claude/                           2w
   6 crates/                            1d
  16 .dockerignore                 96   9w
  18 AGENTS.md                   8.5K   1d
  19 CHANGELOG.md                190K   1d
  NORMAL    main  strop/                                     27 items    3%   1:1
```

```
▌  1 empty folder  ·  o adds a name, :w creates it
  NORMAL    main  demo/empty/                                 0 items  100%   1:1
```

```
   2 lib.rs                         4   2d
▌  3 main.rx                   was main.rs
   4 fresh.txt                 new
   6 notes.txt                 from ../docs/notes.txt
  NORMAL    main  demo/src/ ●                                 7 items  100%   3:7
```

```
▌  1 demo/src/ — 7 changes
   3   rename  main.rs → main.rx
   4   create  fresh.txt
   7   move    ../docs/notes.txt → notes.txt
   8   copy    lib.rs → lib copy.rs
   9   trash   new.rs
  11 y apply   q cancel
```

## 3. Decisions (taken with the design pass's recommendation)

1. `q` records a macro (fidelity; same as collections).
2. No header row; path in modeline and pane title.
3. Always confirm, also for create-only changes: one rule.
4. Size and age; no permissions.
5. Dotfiles shown by default; `g.` hides.
6. Local delete is Trash only; `:!rm` exists.
7. `:browse` removed in favour of `:e dir`.

## 4. Order of work

1. Hygiene: release removes the lock file; listings drop protocol names; the
   read-only allow-list gains the `window-*` ids.
2. Names-only text, always editable; `Enter`/`-`/`g.` routing; cursor kept
   across filter, reload and apply.
3. Confirm buffer and apply flow; delete receipts, verify, recovery, history.
4. Delete `:fs`, `:browse` and the action rows; a "Folder" section in `?`.
5. The look: one row renderer, the modeline summary, goldens.
6. Git marks per entry — deferred to `plans/0028` unless a status snapshot is
   already held outside the status view.
7. A PTY journey over the whole flow; README and CHANGELOG.

## 5. Record

Landed for 0.65.0 (2026-11-13), steps 1–5 and 7. Where it departs from §2–§4:

- **Unconfirmed changes are checked by themselves.** The receipts, history
  commands and `:fs verify` are gone, but the engine still records each
  apply internally: an unconfirmed step blocks its paths and is verified
  on disk at once (`settle_filesystem_attempt` → `verify_filesystem_step`,
  one after another); nothing is retried. `:fs undo` (recovery) is deleted.
- **A cut pasted in another folder is a copy there**, and the deleted row a
  delete in its own folder: each folder has its own draft, so there is no
  cross-folder move in one write. Moving within a folder's subtree is
  `cc sub/name`.
- **`:e` follows vim**: with pending edits it says `:e!`, which drops them.
- **Always-on drafts changed four "has a draft" checks to "has pending
  edits"** (buffer dirty): reload keeps a draft only when dirty; folder
  views are remembered and restored when clean; admission blocks another
  folder's paths only while that folder is dirty or applying; a
  clean folder listed again under an open request keeps the open fresh.
- **Two races found by the PTY journey and fixed**: `Enter` on a renamed
  row right after `y` opened the old path (now `Draft::location`, which
  maps to the applied target while re-listing), and that open was then
  dropped as stale when the re-list landed (`open_fresh`).
- **A moved folder that is open elsewhere lists again at its new path**
  (the old code rewrote its first line as a header).
- **A folder never gets structural indentation**: `o` used to queue the
  code indenter behind a job.
- Step 6 (git marks per entry) is not done: no status snapshot is held
  outside the status view. Recorded in `plans/0028`.
- The remote batch refuses a copy and a rename of the same file in one
  write ("multiple conflicting operations own one source"); the SSH
  journey keeps them on separate files. Lifting that is a strop-fs change.

Gates: `cargo test` (engine 1,108, binary 261, PTY journey `folder.rs`),
clippy `-D warnings`, the real-sshd lane (`remote_ssh` 9, `ssh_exec` 6),
goldens `folder`, `folder-empty`, `folder-filter`, `folder-pending`,
`folder-confirm` in hone and vellum at 140×40 and 80×24.
