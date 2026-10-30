# The filesystem workspace

Open a directory with `strop DIRECTORY`, `:browse`, or `:e DIRECTORY`.
`Space e` reveals the current source in its parent. Browsing keeps the
process cwd unchanged; Enter opens a structured entry and `-`/Backspace
goes to its parent. Directory-local `Space a` opens filesystem actions.

Filesystem mutations use **Review → Apply → Receipt**, locally and on SSH
hosts where the native worker is admitted (restricted/SFTP-only hosts
remain read-only):

```vim
:fs create nested/new.txt
:fs mkdir new-directory
:fs rename new-name.txt
:fs move destination-directory
:fs copy stored stored-copy.txt
:fs copy buffer edited-copy.txt
:fs trash
:fs remove
:apply-change
:cancel-change
:fs operations
:fs verify OPERATION STEP
:fs undo OPERATION STEP
```

Relative filesystem destinations use the current Directory, or the current
source's parent; source-less buffers use the workspace root. Explicit
resource URIs select another namespace. Creation never truncates an
occupied name; rename/move is no-clobber. Review lists missing-parent
creation explicitly. Stored-copy and buffer-copy select different versions
without saving the source. Trash uses native recovery facilities where
supported; remote Trash is unavailable, not an alias for permanent removal.
Recursive removal of non-empty directories and unsafe alias/root operations
are refused.

## The filename draft

`:fs edit` enters a name-only **filename draft**. Ordinary motions, edits,
whole-line yank/paste, undo/redo and macros change the draft, not disk.
`:w` prepares review; `:apply-change` performs filesystem changes;
`:cancel-change` returns to the intact draft. Browsing or refreshing the
directory retains edited names and source provenance. Use `:fs discard` to
discard explicitly. Copies need a distinct name; ambiguous joins and
conflicting targets refuse instead of guessing which file a row represents.

## Relocation and barriers

Confirmed relocation keeps source text, history and document identity.
Remote relocation revokes old write authority; use `:remote edit` again
before saving the renamed buffer. Pending saves and unconfirmed operations
block conflicting work. Receipts survive leaving the view; verification
never blindly retries a mutation. Containers remain read-only through these
actions.

Local saves/save-as and local filesystem mutations use a conservative
per-editor write barrier, including parent aliases. Let an admitted save
finish before applying a local filesystem review, and resolve unconfirmed
receipts before saving. Move verification and recovery require the recorded
post-publication version: finding the same inode at a pathname is not
sufficient ownership evidence. Receipts are in-memory, not crash recovery.
Graceful global exit refuses unresolved outcomes; explicit `:qa!` reports
them and exits unsuccessfully rather than claiming they were resolved or
persisted.

Removing an otherwise empty directory may retire its validated, quiescent
protocol locks. Active locks, unknown entries and other directory contents
cause refusal; partial housekeeping is reported even when cancellation
prevents removal.

## Indentation controls

`:tab-size` opens source-specific width/style controls; `:tab-size 3`,
`:tab-size auto`, and `:indent-style tabs|spaces|auto` are direct forms.
Changing indentation settings never rewrites existing bytes. The modeline
names the effective input owner and source setting. Matching delimiters use
the same cancellable source resolver as `%`, without moving the view. The
indentation engine itself is described in [editor.md](editor.md#indentation).
