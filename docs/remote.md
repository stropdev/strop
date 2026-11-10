# SSH and remote editing

Remote files are real buffers with their own identity, never a local
lookalike. strop reaches a host with plain OpenSSH: nothing is installed
there.

```sh
strop ssh://devbox/etc/
strop +120 ssh://user@devbox/var/log/app.log
strop remote "ssh://user@devbox:2222/~/project/src/lib.rs"
```

`strop remote URL` and `--remote URL` are the same command — see
[cli.md](cli.md). `-R` opens read-only, as for a local file.

## Connecting

Press **Space o** or run **`:remote`** to choose a known or connected
destination, or select **Add a host** and enter `host`,
`user@host:port` or a complete SSH path. Opening the chooser does not
authenticate; Enter connects. New hosts start at `/`; directories you
choose are remembered privately. `:remote home` browses the home
directory the server reports; `:remote root` returns to `/`.

```vim
:browse ssh://devbox/
:filter app
:remote connect ssh://devbox
:remote list
:remote disconnect ssh://devbox
:remote clear
```

**A connected host is editable.** Logging in is the one consent: there is
no separate step to edit files, run Git, search or open a terminal there.

OpenSSH supplies aliases, keys, agent and `ProxyJump` from your
`~/.ssh/config`. Authentication is noninteractive: host keys must already
be trusted and a key or agent must answer without a prompt; otherwise the
refusal carries ssh's own words and a hint. strop runs one OpenSSH master
per host (`ControlMaster`, socket in a private directory under
`$XDG_RUNTIME_DIR`), so every later read, save, search or terminal is a
channel on it rather than a new login. Masters strop started stop when it
exits; after a crash they expire on their own after ten idle minutes.

Directory listings are searchable real buffers with kind, POSIX
permissions, server-reported byte size and native filename columns.
Missing attributes show `?`. Listings include hidden entries and do not
evaluate ignore files. Enter opens an entry; `-`, Backspace or `../`
returns to the parent and restores the selected child. Tab completes SSH
hosts and paths without starting authentication: remote candidates need
a live connection or a cached listing. Percent-encode reserved path
bytes (`%20`, `%23`, `%25`); native Unix filenames stay intact. Files are
read whole, up to 256 MiB of UTF-8 text.

## Saving

**`:w`** uploads the buffer to a temporary name beside the file, gives it
the file's permission bits and renames it over the original, so readers
never see a half-written file. A symlink is saved through to its target.

- If the file on the host changed since it was read (its size or
  modification time moved), `:w` is refused and your edits stay:
  **`:e!`** reloads the host's version, **`:w!`** replaces it anyway.
- **`:w PATH`** or **`:w ssh://…`** writes a copy (a relative path is
  beside the file) and never replaces an existing file; `:w! PATH` does.
- Saves run off the input thread; a reply for an older revision never
  clears newer unsaved edits.

Limits, stated plainly: there is no cross-process lock (as in vim, a
writer racing the final check wins); SFTP modification times are whole
seconds, so a same-size rewrite within the second of the read goes
unnoticed; ownership is not carried over when another user's file is
replaced.

## File management

`:fs create`, `:fs mkdir`, `:fs rename`/`move`, `:fs copy` and
`:fs remove` (and filename drafts in a remote directory) run on the host
through the same Review → Apply → Receipt flow as locally — see
[filesystem.md](filesystem.md#ssh-hosts). There is no Trash on a host
(removal is permanent), nothing is ever replaced, directories are not
copied recursively, and nothing is atomic across steps.

## Search, Git, language servers, terminal

They run **on the host**, as commands on the connection:

- **`Space /`** searches the remote workspace with `rg` on the host.
- **Git** signs, log (`Space g l`), blame and diffs run `git` there.
- A language server starts on the host when it is installed there; the
  configuration comes from your local `languages.toml` and the project's
  `.strop/languages.toml` on the host.
- **`:terminal`** and **Space t** in a remote file or directory open your
  login shell on the host in that directory (`ssh -tt` on a local PTY, so
  Ctrl-C and window size work; keys typed before the shell answers are
  kept). `:terminal-local` is the explicit local alternative.

`rg`, `git` and the language server must be installed on the host. Each
command runs through your login shell there (sshd's `$SHELL -c`), so a
heavy shell startup file slows every search and Git query; csh-family
login shells are not supported for commands with special characters.

## Containers

Strop does not attach to containers. Open **`:terminal`** and run
`docker exec -it <container> sh` to work inside one.
