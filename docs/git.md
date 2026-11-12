# Git

Everything git is under `Space g`. Each git view is an ordinary read-only
buffer: motions, `/`, yank and counts work, `q` closes it.

| key | does |
|---|---|
| `Space g g` | status: the changed files |
| `Space g c` | commit |
| `Space g b` | branches: switch or create |
| `Space g l` | log |
| `Space g h` | history of this file (in visual mode: of the selected lines) |
| `Space g B` | blame: who last changed each line, in the gutter; again to hide it |
| `Space g s` `Space g u` `Space g X` | stage, unstage, discard the hunk under the cursor |
| `Space g p` | preview the hunk under the cursor |
| `Space g y` `Space g o` | copy / open a permalink to the line or selection |
| `]c` `[c` | next / previous hunk |

The same letters mean the same thing everywhere: `s` stages, `u` unstages,
`X` discards.

## Status — `Space g g`

```text
Head: main
Upstream: origin/main, ahead 1

Staged (1)
A docs/notes.md

Unstaged (2)
M src/lib.rs
@@ -1,3 +1,4 @@
 fn a() {}
+fn b() {}
 fn c() {}
M src/main.rs

Untracked (1)
? scratch.txt
```

The keys act on the row under the cursor. On a file row that is the file; on
a diff row it is that hunk; on a section heading it is every file in the
section.

| key | does |
|---|---|
| `enter` | open the file — at the line, from a diff row |
| `s` | stage |
| `u` | unstage |
| `-` | toggle: stage what is unstaged, unstage what is staged |
| `=` | show or hide the file's diff under its row |
| `]c` `[c` | next / previous hunk (next / previous file while no diff is shown) |
| `X` | discard, after `y` confirms; any other key cancels |
| `cc` | commit |
| `ca` | amend the last commit |
| `q` | close |

- **Discard** restores an unstaged file from the index, deletes an untracked
  file or directory, or reverts one unstaged hunk in the working file. It
  refuses on a staged or conflicted row — unstage first — so one key never
  throws away two layers of work. A buffer open on a discarded file reloads.
- **Conflicts** get their own section at the top. Resolve the file, then `s`
  on its row marks it resolved.
- The list keeps itself up to date: it refreshes after every stage,
  unstage, discard and commit, when files change on disk, when the terminal
  window regains focus, and when you press `Space g g` again. The cursor stays on
  the same file or hunk.
- Renames show as a deleted file and an added file.
- The [lock files](filesystem.md#lock-files) strop leaves beside files it
  saved are never listed.

## Commit — `Space g c`, or `cc` in the status buffer

The message is edited in `.git/COMMIT_EDITMSG`, an ordinary file buffer. It
opens with an empty first line for the message and `#` lines that list what
the commit will hold; `#` lines are left out of the message.

- `:wq` commits. So does `:w` followed by `:q`.
- `:q!` without writing aborts.
- `ca` (or amending from the status buffer) starts from the last commit's
  message and needs nothing staged.

The commit is made by `git commit` itself, so your hooks, signing and
`user.*` configuration all apply. A hook that fails stops the commit and its
output is shown; an empty message is refused by git. A signing setup that
needs to ask for a passphrase in the terminal can not do so here — use an
agent, or commit from a shell (`Space t`).

With nothing staged, `Space g c` says so and opens nothing.

## Branches — `Space g b`

A picker of local branches (the current one first) and then remote branches.

- `enter` switches. A remote branch such as `origin/topic` switches to
  `topic`, creating it to track the remote if it does not exist.
- The last row, **new branch from the typed name**, creates a branch named by
  what you typed in the filter and switches to it.

Git decides whether the switch is allowed. If your changes would be
overwritten it refuses and its message is shown; nothing is changed. After a
switch, unmodified buffers show the new branch's files; buffers with unsaved
changes are kept and marked as changed on disk.

## Hunks in a file

The gutter marks the lines that differ from what is staged. `]c` / `[c` move between
hunks, `Space g p` previews one, `Space g s` stages it, `Space g u` unstages
the staged hunk under the cursor, and `Space g X` discards it in the buffer
(an ordinary edit: `u` undoes it). Staging reads the file on disk, so save
first.

## Log, history, blame

- `Space g l` is the commit graph. `enter` on a commit lists its files;
  `enter` on a file shows its diff, with the commit's other files in a
  sidebar (`tab` moves focus, `]f` / `[f` step through files).
- `Space g h` is the log of the current file; from a visual selection, the
  history of those lines.
- `Space g B` shows who last changed each line, in the gutter; press it
  again to hide it. `enter` on a line opens the log at that commit. The
  gutter hides while the buffer has edits made after blame ran. In a
  read-only buffer (a file opened with `-R`, a git view) there is no
  gutter; a card shows the blame for the current line instead.

### Large repositories

Without a commit-graph file, `git log --graph` reads every commit before it
shows the first: about half a second on 150,000 commits, and twice that
for one file's history. The first time the log or a file's history takes
longer than a tenth of a second, strop writes a commit-graph for that
repository in the background, and every later log is fast — about 10 ms,
with a file's history showing its first rows within tens of milliseconds
and the rest as git finds them.

The file goes in strop's own state directory,
`$XDG_STATE_HOME/strop/graphs/` (`~/.local/state/strop/graphs/` when that
is not set). Your repository is not modified: nothing is written into
`.git`, no lock is taken. It is used only for the log and file history;
git checks it against the repository, so an out-of-date one never changes
what the log shows. strop keeps graphs for the 16 repositories used most
recently. A repository with a commit-graph of its own (`git gc` writes
one) gets none from strop. Setting `GIT_ALTERNATE_OBJECT_DIRECTORIES`
yourself turns this off.

## SSH projects

On an SSH host ([remote.md](remote.md)) the gutter marks, log, blame,
history and diffs work. Status, commit and the branch picker do not work
there yet; they say so and do nothing. Staging and discarding hunks do not
work there either.
