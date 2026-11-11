# Files and directories

Browse a directory, and create, rename, move, copy and delete files without
leaving strop.

Open a directory with `strop DIRECTORY`, `:browse` or `:e DIRECTORY`.
`Space e` opens the directory of the current file with the cursor on it.
Enter opens the entry under the cursor; `-` or Backspace goes up. Browsing
does not change the project root. Each row shows the name, the size, and
the kind and permissions; `:filter TEXT` narrows the listing and `:filter`
alone shows everything again.

In a directory listing `Space a` lists the file operations you can do
there; `:fs` with no verb lists them from anywhere. In a file, `Space a` is
about the code: the language server's actions, and anything a failed save
or connection left for you to deal with.

## File operations

Nothing changes on disk until you confirm. Each `:fs` command opens a
review that lists the exact steps. `Enter` (or `:fs apply`) does them; `q`
(or `:fs cancel`) drops them. This works the same on your machine and on an
SSH host (see [SSH hosts](#ssh-hosts) for what differs there).

```vim
:fs create nested/new.txt
:fs mkdir new-directory
:fs rename new-name.txt
:fs move destination-directory
:fs copy stored stored-copy.txt
:fs copy buffer edited-copy.txt
:fs trash
:fs remove
:fs apply
:fs cancel
:fs operations
:fs verify OPERATION STEP
:fs undo OPERATION STEP
```

In a listing, the commands act on the entry under the cursor, on the lines
of a range (`:'<,'>fs trash`), or on the entries you marked with `:fs mark`.

A relative name is taken from the directory you are browsing, or from the
directory of the current file. In a buffer with no file it is taken from
the project root. A destination can also be an `ssh://` address, but one
review works on one machine: strop does not copy or move between your
machine and a host, or between two hosts.

strop never overwrites: creating, renaming or moving onto a name that
exists is refused. If parent directories have to be created, the review
lists each one. `:fs copy stored` copies the file as it is on disk;
`:fs copy buffer` copies the text in the buffer, unsaved edits included.
Neither saves the file. Only single files are copied, not directories.

`:fs trash` moves the file to your system's trash. On an SSH host there is
no trash, and `:fs trash` is refused; it never deletes instead.
`:fs remove` deletes for good, and the review says so. It will not remove
a directory that still has files in it. strop also refuses to rename or
remove `/`, the project directory or a directory that contains it, and
symbolic links, hard-linked files and special files.

Afterwards strop keeps a record of each step and whether it succeeded:

- `:fs operations` lists what was done in this run of strop. Operations
  and their steps are numbered; those are the OPERATION and STEP numbers
  the next two commands take. Each step is marked committed, refused,
  cancelled or unconfirmed, with the command to use next.
- `:fs verify OPERATION STEP` checks the disk to see whether a step really
  happened. Use it when a step is **unconfirmed**: strop could not tell
  whether it went through, for example because the connection dropped
  before the answer came. It only looks; it never does the step again.
- `:fs undo OPERATION STEP` opens a review of the reverse step: rename
  back, restore from the trash, or remove what was created or copied. A
  permanent removal cannot be undone.

## Renaming by editing the list

`:fs edit` makes the directory listing editable: change the names as text.
Motions, edits, yanking and pasting whole lines, undo and macros all work,
and nothing on disk changes yet. `:w` opens the review; `Enter` (or
`:fs apply`) does the changes; `:fs cancel` goes back to your edited list.
Your edits survive a refresh or a visit to another directory. `:fs discard`
throws them away.

- Change a name to rename the entry.
- Add a line to create a file; end the name with `/` for a directory.
- Delete a line to remove the entry. On your machine it goes to the trash.
  On an SSH host you first have to say `:fs deletes permanent`.
- Yank a line and paste it to copy the file. The copy needs a new name.
- Delete a line and paste it back to keep the entry: that is a move, not
  a removal.

If two lines end up with the same name, or strop cannot tell which file a
line stands for (two entries joined with `J`, one name split over two
lines), it refuses and names the line; undo that edit or `:fs discard`. A
listing of more than 512 entries has to be filtered before `:fs edit`.

## Open files, saves in progress, and quitting

When you rename or move a file that is open, the buffer follows it and
keeps its text, unsaved edits and undo history.

On your own machine strop does one disk change at a time. While a save is
still running, a file operation is refused with a message: wait for the
save, then confirm again. While a file operation is running or one of its
steps is unconfirmed, `:w` is refused the same way: settle the step with
`:fs verify`, then save. Nothing is queued behind your back.

`:fs verify` and `:fs undo` on a moved file act only if the file is still
exactly as strop left it. If another program changed or replaced it since,
they refuse and say so.

The record of steps stays after you leave the review, but it is lost when
strop exits or crashes. `:qa` refuses to quit while a step is unconfirmed.
`:qa!` quits, lists those steps on the terminal, and exits with an error
status.

### Lock files

When strop saves a file or does a file operation on your machine, it
leaves a small lock file beside it, named `.strop-lock-` followed by a
long string of letters and digits. Files (`Space f`), Search and git status never list them; a
directory listing does. A directory that holds only leftover lock files
counts as empty: removing it deletes them too, and the record says how
many. A lock that is in use, or anything else in the directory, stops the
removal.

## SSH hosts

On an SSH host the same commands work, with the same review first and the
same record afterwards: create files and directories, rename, move, copy a
file (as it is on the host, or the text in your buffer) and remove. What
differs, because of how SFTP works:

- **No trash.** `:fs trash` is refused on a host; `:fs remove` deletes for
  good, and the review says so.
- **Nothing is overwritten.** A name that is taken is refused in the
  review, and checked again just before each step runs. If the name
  appears on the host between the review and the step, the step fails
  instead of overwriting. A file that changed on the host since the review
  is refused too.
- **Directories are not copied**; copy files one at a time, or use a
  terminal on the host. Only an empty directory can be removed.
- **Steps are not all-or-nothing.** If step 3 of 5 fails, steps 1 and 2
  stay done. Nothing stops another program on the host from changing the
  same files. If the connection drops before the host answers, the step is
  shown as unconfirmed. `:fs verify` checks the host to see whether it
  happened. It never runs the step again.
- A copy keeps the file's permissions, not its owner or times. New files
  and directories get 0644 and 0755, less the host's umask.

Indent width and tabs or spaces (`:tab-size`, `:indent-style`) are in
[editor.md](editor.md#indent-width-and-tabs-or-spaces).
