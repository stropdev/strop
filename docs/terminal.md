# Terminals

strop has a built-in terminal that works like Neovim's `:terminal`. Run a
shell in a pane, leave it with `Ctrl-\ Ctrl-N`, and move, search and yank
in its output with the usual keys.

| Keys | Effect |
| --- | --- |
| `Space t` | new shell in a split below, in TERMINAL mode |
| `Space T` | new shell in the current pane (= `:terminal`) |
| `:terminal [CMD]`, `:term` | new shell, or run `CMD` and keep its output |
| `Ctrl-\ Ctrl-N` | TERMINAL → NORMAL (the only way out) |
| `i` `I` `a` `A` | NORMAL → TERMINAL, at the program's cursor |
| `:q` | hide a running terminal; `Space b` returns to it |
| `:bd!` | stop the program and delete the buffer |
| `:qa` | refuses while terminals run; `:qa!` stops them and quits |
| `:r !CMD` | insert a command's output below the cursor (as in Vim) |

**TERMINAL mode** sends every key to the program: Esc, Ctrl-C, Ctrl-W, Alt
and function keys included. The mode chip reads `TERMINAL`, and the status
line shows the one way out. Some terminals send `Ctrl-\` as the byte `0x1c`
(what Vim calls `Ctrl-4`); it is the same key. A paste arrives the way any
terminal would send it: bracketed when the program asked for bracketed
paste, as plain typing otherwise.

**NORMAL mode** is a live, read-only buffer of the screen and its
scrollback: the usual motions, search, selection and yank work on it, and
output keeps arriving while you read. The view follows the output while the
cursor is on the last line. `k`, `Ctrl-U` or a search stop it following and
`G` starts it again, exactly as in Neovim.

Scrollback holds 10,000 lines; older lines are dropped. When they are, your
cursor and marks stay on the same text. Up to 16 terminals can be open at
once.

A terminal buffer is named `term://{directory}//{N}:{program}` (`N` counts
terminals from 1) in `Space b`, pane titles and the status line. Switching
away and back (`Ctrl-^`, `Space b`, pane focus) restores the cursor and the
scroll position and lands in NORMAL mode. A terminal whose program has
exited stays readable, with its exit status; `:terminal` starts a new one.

The shell is `$SHELL` (or `/bin/sh`), started in the project directory
(`:pwd`). In a directory listing, `Space a` → **Terminal here** starts it
in that directory. In a file or directory on a connected SSH host, the
terminal is your login shell on that host, and its name shows the host.
Ctrl-C and the window size reach it. If the host is not connected, strop
says so and does not open a shell on your own machine by mistake.
`:terminal-local` opens a shell on your machine on purpose. The directory
in the terminal's name is where it started; it does not change when you
`cd`.

A program running in the terminal cannot change your real terminal's title
or window size, and cannot read or write your clipboard. Your own `y` still
copies. `:w!` and `:set noro` do not turn a terminal into a file: yank the
text into an ordinary buffer to keep it. Closing a split does not stop the
program in it. A program that put itself in the background (`nohup`,
`setsid`) can keep running after its terminal is closed.

When one terminal is shown in two panes, the pane you type in sets its
size. The prebuilt strop needs nothing else installed. To build strop from
source you need Zig 0.16.0.
