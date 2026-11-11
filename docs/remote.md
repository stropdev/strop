# SSH and remote editing

Edit files on another machine over SSH. strop uses the `ssh` already on
your machine. Nothing is installed on the host.

```sh
strop ssh://devbox/etc/
strop +120 ssh://user@devbox/var/log/app.log
strop remote "ssh://user@devbox:2222/~/project/src/lib.rs"
```

`strop remote URL` and `--remote URL` are the same command (see
[cli.md](cli.md)). `-R` opens read-only, as for a local file.

## Connecting

Press **Space o** or run **`:remote`** to pick a host: the directories you
opened before, the hosts you are connected to, and the hosts in your SSH
config and `known_hosts`. Or select **Add a host…** and enter `host`,
`user@host:port` or a complete `ssh://` address. Opening the list does not
log in; Enter connects. A new host starts at `/`. strop remembers, on this
machine only, the directory you last opened on each host (in
`remote-destinations.json` under `$XDG_STATE_HOME/strop`, normally
`~/.local/state/strop`).

```vim
:browse ssh://devbox/
:filter app
:remote connect ssh://devbox
:remote list
:remote disconnect ssh://devbox
:remote clear
```

- `:browse ssh://HOST/DIR` lists a directory; `:filter TEXT` narrows the
  listing.
- `:remote home` lists your home directory on the current host;
  `:remote root` goes to `/`.
- `:remote connect` logs in and keeps the connection; `:remote disconnect`
  closes it (the current host when no address is given); `:remote clear`
  closes every connection. The remembered directories stay.
- `:remote list` shows the open connections.

**Once connected, you can edit.** There is no second confirmation before
you save files, search, use Git or open a terminal on the host.

Host aliases, keys, the agent and `ProxyJump` come from your
`~/.ssh/config`. strop cannot ask you for a password or a passphrase, and
cannot ask you to accept a new host key. Log in once with plain `ssh host`
to accept the key, and use a key or an agent. If login fails, strop shows
ssh's own error and a hint.

strop logs in once per host and reuses that connection for every read,
save, search and terminal, so you are not asked to log in again. The
connection closes when strop exits. After a crash it closes by itself after
ten idle minutes. (It is an OpenSSH `ControlMaster`; its socket is in a
private directory under `$XDG_RUNTIME_DIR`.)

A directory listing is an ordinary buffer: move and search in it as usual.
Each row shows the name, the size, and the kind and permissions; `?` stands
for something the host did not report. Hidden files are shown, and
`.gitignore` is not applied. Enter opens an entry; `-`, Backspace or `../`
goes up and puts the cursor back on the directory you came from. Tab
completes host names and paths without logging in; paths on a host complete
only once you are connected or have listed that directory before. In an
`ssh://` address write a space as `%20`, `#` as `%23`, `?` as `%3F` and `%`
as `%25`. A file is read whole and can be up to 256 MiB of UTF-8 text.

## Saving

**`:w`** uploads the buffer to a temporary name beside the file, gives it
the file's permissions and renames it over the original, so nobody ever
reads a half-written file. A symlink is saved through to the file it points
at.

- If the file on the host changed since it was read (its size or
  modification time is different), `:w` is refused and your edits stay:
  **`:e!`** reloads the host's version, **`:w!`** replaces it anyway.
- **`:w PATH`** or **`:w ssh://…`** writes a copy (a relative path is
  beside the file) and never replaces an existing file; `:w! PATH` does.
- You can keep typing while a save is on its way. If you edit during the
  save, the buffer stays marked as modified.

What strop cannot protect against: if another program writes the file in
the instant between strop's last check and its save, the later write wins,
as in Vim. SFTP reports modification times in whole seconds, so a rewrite
of the same size within the same second as your read goes unnoticed. When
you replace a file that belongs to another user, the new file belongs to
you.

## File management

`:fs create`, `:fs mkdir`, `:fs rename`/`move`, `:fs copy` and `:fs remove`
(and renaming by editing the list) work on the host as they do on your
machine: a review first, then `Enter` to do it. See
[filesystem.md](filesystem.md#ssh-hosts). On a host there is no trash
(removal is for good), an existing file is never overwritten, a directory
is not copied, and if a later step fails the earlier ones stay done.

## Search, Git, language servers, terminal

They run **on the host**:

- **`Space /`** searches the project on the host, using the host's `rg`.
  Replace across files does not work on a host yet.
- **Git**: the change marks in the gutter, the log (`Space g l`), blame and
  diffs use the host's `git`. Status, commit and the branch picker work
  only on your own machine for now ([git.md](git.md)).
- A language server starts on the host when it is installed there. Its
  settings come from your local `languages.toml` and from the project's
  `.strop/languages.toml` on the host.
- **`:terminal`** and **Space t** in a remote file or directory open your
  login shell on the host. Ctrl-C and the window size work, and keys typed
  before the shell answers are kept. `:terminal-local` opens a shell on
  your own machine instead. To work inside a container, open a terminal
  and run `docker exec -it <container> sh`.

`rg`, `git` and the language server must be installed on the host. Each
command runs through your login shell there, so a slow shell startup file
slows every search and Git query. csh-family login shells are not
supported for commands with special characters.
