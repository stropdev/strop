# Folders

A folder is a buffer whose lines are its entries' names. Move in it with
the usual keys; change files by editing the names and writing.

Open one with `strop .`, `strop FOLDER` or `:e FOLDER`. `Space e` opens the
folder of the current file with the cursor on it. Opening a folder does not
change the project root.

```
▌  1 crates/                            1d
   2 docs/                              2w
  18 AGENTS.md                   8.5K   1d
  19 CHANGELOG.md                190K   1d
  NORMAL    main  strop/                                   27 items    3%   1:1
```

Folders come first, then files; a folder ends in `/`. Beside each name
are its size (none for a folder) and how long ago it changed. They are not
part of the text: `yy` yanks just the name. The modeline shows the folder,
relative to the project like a file, and how many entries it lists.

| Key | Does |
|---|---|
| `Enter` | open the file or folder under the cursor |
| `-` | the parent folder, with the cursor on the folder you left |
| `g.` | hide or show dotfiles (each folder remembers) |
| `:filter TEXT` | list only matching names; `:filter` alone lists all |
| `:e` | list the folder again |
| `Space a` | search in this folder, a terminal in it, or copy its path |
| `Ctrl-O` | back to where you came from |

## Changing files

Edit the names, then `:w`:

- change a name (`cw`, `cc`, `r`, a macro, `:s`) to rename it; give it a
  path such as `sub/name.txt` to move it into a folder;
- add a line (`o`) to create a file; end it with `/` for a folder, and
  missing parent folders are created too;
- delete a line (`dd`) to delete the entry;
- yank a line and paste it (`yyp`), then give the copy a new name, to copy
  a file.

Each changed row says what writing it will do: `was main.rs`, `new`,
`copy of lib.rs`. `u` undoes an edit; `:e!` drops all of them and lists
the folder again.

`:w` shows the changes before anything happens:

```
▌  1 demo/src/ — 4 changes

     rename  main.rs → main.rx
     create  fresh.txt
     copy    lib.rs → lib copy.rs
     trash   old.rs

   y apply   q cancel
```

`y` (or `Enter`) makes them; `q` (or `n`) goes back to the folder with
your edits still there. When every change is made the folder comes back,
listed again, with the cursor where it was. If one fails, the list stays
and says what happened to each change; `q` returns to the folder.

Rules that do not change:

- Nothing is overwritten. A name that already exists is refused.
- A deleted file goes to your system's Trash. On an SSH host there is no
  Trash: the delete is permanent, and the list says `delete`.
- A copy copies the file as it is on disk. If the file is open with
  unsaved changes, write it first; strop says so instead of guessing.
- Only files are copied, not folders. Only an empty folder is deleted on
  an SSH host.
- strop refuses to rename or delete `/`, the project folder or a folder
  that contains it, and symbolic links, hard-linked files and special
  files.
- A folder of more than 512 entries is read-only; `:filter` it down to
  edit the names.
- A name yanked in one folder and pasted in another is a copy there; the
  line you deleted is a delete in its own folder when you write that.

When you rename or move a file that is open, the buffer follows it and
keeps its text, unsaved edits and undo history. While another folder has
unwritten edits that touch the same paths, writing is refused until you
write or drop those.

## Saves in progress, and unconfirmed changes

On your own machine strop makes one disk change at a time: while a save
is running, writing a folder is refused (and the other way round). Wait,
then write again. Nothing is queued behind your back.

If strop cannot tell whether a change went through (an SSH connection
dropped before the host answered), the change is shown as unconfirmed and
strop checks the disk for it on its own; it never makes the change twice.
`:qa` refuses to quit while a change is unconfirmed; `:qa!` quits, lists
those changes on the terminal, and exits with an error status.

strop keeps a lock beside a file while it writes or changes it, named
`.strop-lock-` followed by a long string of letters and digits, and
removes it when done. Folders, Files (`Space f`), Search and git status
never show them. One left by a crash is removed with its folder.

## SSH hosts

`strop ssh://HOST/DIR/` or `:e ssh://HOST/DIR/` opens a folder on a host;
everything above works the same, over SFTP. What differs:

- **No Trash.** A delete is permanent, and the list says `delete`.
- **Nothing is overwritten.** Each name is checked again just before its
  change runs; one that appeared on the host since `:w` fails instead.
- **Changes are not all-or-nothing.** If the third of five fails, the
  first two stay made. Nothing stops another program on the host from
  changing the same files.
- A copy keeps the file's permissions, not its owner or times. New files
  and folders get 0644 and 0755, less the host's umask.
- One write works on one machine: strop does not copy or move between
  your machine and a host, or between two hosts.

Indent width and tabs or spaces (`:tab-size`, `:indent-style`) are in
[editor.md](editor.md#indent-width-and-tabs-or-spaces).
