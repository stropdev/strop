# Vim compatibility

Generated from the command table (`strop compat`).
`✓` works today; `(soon)` is planned. What Neovim binds and strop
does not is listed at the end, under Missing and Reassigned.

## Moving

- `✓ h j k l` — move (never off the line)
- `✓ w b e W B E` — word / WORD motions
- `✓ zz zt zb` — center/top/bottom the cursor line
- `✓ H M L` — top/middle/bottom visible line
- `✓ g;` — older change (changelist)
- `✓ g,` — newer change (changelist)
- `✓ ge gE { }` — word-end back / paragraph motions
- `✓ - +` — line up / down, on the first non-blank
- `✓ ( )` — sentence back / forward
- `✓ [( [{ ]) ]}` — the unmatched bracket before / after
- `✓ ]] [[` — help: next / previous section
- `✓ g_ g0 g^ g$ gj gk` — last non-blank; line start, first non-blank, end; line down / up
- `✓ gn gN` — select the next / previous search match (cgn then . changes each)
- `✓ 0 $ G %` — line/file/pair jumps
- `✓ gg` — top of file
- `✓ enter` — line down, first non-blank (collection: open the file here; blame: the line's commit; help: the section under the cursor)
- `✓ tab` — jump list forward (ctrl-i)
- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ ctrl-e ctrl-y` — scroll the window one line down / up
- `✓ ctrl-^` — alternate buffer
- `✓ grr` — references (LSP)
- `✓ grn` — rename the symbol under the cursor (prompts with its name)
- `✓ gra` — actions here, as space a (code actions; Neovim's key)
- `✓ gri gI` — implementation (LSP)
- `✓ gy` — type definition (LSP)
- `✓ gD` — declaration (LSP)
- `✓ ]d` — next diagnostic
- `✓ [d` — previous diagnostic
- `✓ gd` — goto definition (LSP)
- `✓ gs` — switch source/header (clangd)
- `✓ f<c> F<c> t<c> T<c>` — find/till char (candidates light up)
- `✓ ]q` — next hit of the collection (in it: from the cursor)
- `✓ [q` — previous hit of the collection (in it: from the cursor)
- `✓ ]f` — next file card (collections)
- `✓ [f` — previous file card (collections)
- `✓ ]c` — next git hunk
- `✓ [c` — previous git hunk
- `✓ m<a>` — set mark at cursor
- `✓ '<a>` — jump to mark: its line ('' is where the last jump left from)
- `✓ `<a>` — jump to mark: its exact position
- `✓ ; ,` — repeat find, same / reversed
- `✓ |` — column motion (vim); pipe moved to space |
- `✓ ^ _` — first non-blank

## Editing

- `✓ ZZ` — write + close window
- `✓ ZQ` — close window without writing
- `✓ gi` — insert at last insert
- `✓ gb` — select next occurrence of word/selection
- `✓ gB` — select all occurrences in buffer
- `✓ ctrl-r` — redo
- `✓ q<a>` — record macro into register
- `✓ @<a>` — play macro (count repeats); @: repeats the last : command
- `✓ Q` — toggle cursor at point (multicursor)
- `✓ d y c > < gU gu g~ g? ! gc` — operators + motion/object: delete, yank, change, indent, case, rot13, filter (opens :{range}!), comment
- `✓ dd yy cc` — an operator doubled acts on whole lines (>> << gUU guu g~~ g?? gcc)
- `✓ D` — delete to line end
- `✓ C` — change to line end
- `✓ Y` — yank to line end
- `✓ s` — substitute char
- `✓ x` — delete char
- `✓ X` — delete char back
- `✓ iw i" i' i( i[ i{` — inner objects (quotes scan the line)
- `✓ ds" cs"' ysiw"` — surround: delete / change / add
- `✓ i a A o O I` — insert (auto-indent)
- `✓ p P` — paste after / before
- `✓ r<c>` — replace char
- `✓ J` — join lines
- `✓ gJ` — join lines as they are, without a space
- `✓ .` — repeat the last change (a count replaces its count; Visual: same extent from the cursor)
- `✓ ~` — toggle case and advance
- `✓ R` — replace mode: type over the text
- `✓ ctrl-a` — add to the number at or after the cursor
- `✓ ctrl-x` — subtract from the number at or after the cursor
- `✓ gp gP` — paste and leave the cursor after the text
- `✓ ]p` — paste lines below at the current indent
- `✓ [p` — paste lines above at the current indent
- `✓ &` — repeat the last :s on this line, with its flags
- `✓ g&` — repeat the last :s on every line, with its flags
- `✓ S` — change line
- `✓ u ctrl-r` — undo / redo (one unit per command)
- `✓ "+y "+p "+P` — system clipboard: yank / paste after / before
- `✓ "xy "xp` — named register: yank / paste
- `✓ gv` — reselect last visual

## Insert

- `✓ ctrl-space` — code completion: language service + current source words
- `✓ ctrl-x ctrl-o` — language-service completion
- `✓ ctrl-n ctrl-p` — source words; choose next/previous completion
- `✓ up down` — choose completion when open; otherwise move the caret
- `✓ ctrl-y` — accept choice
- `✓ ctrl-e` — dismiss
- `✓ tab s-tab` — indent/dedent; with the menu open, cycle candidates with a live preview
- `✓ esc` — dismiss completion and enter Normal mode in one event (one undo unit)
- `✓ backspace` — delete back
- `✓ enter` — new line (auto-indent); accept completion only after deliberate selection
- `✓ } ] )` — closer on indent-only line aligns to its opener

## Visual

- `✓ gb` — select next occurrence of word/selection
- `✓ gB` — select all occurrences in buffer
- `✓ gv` — reselect last visual
- `✓ ctrl-v` — visual block
- `✓ v V` — visual / visual-line (in Visual: switch, or leave)
- `✓ d x` — delete the selection
- `✓ y` — yank the selection
- `✓ c s` — change the selection
- `✓ >` — indent selection (linewise: count levels)
- `✓ <` — dedent selection (linewise: count levels)
- `✓ U gU` — uppercase the selection
- `✓ u gu` — lowercase the selection
- `✓ ~ g~` — toggle the selection's case
- `✓ g?` — rot13 the selection
- `✓ gc` — toggle comments on the selected lines
- `✓ !` — filter the selected lines through a shell command
- `✓ D X` — delete the selected lines
- `✓ Y` — yank the selected lines
- `✓ C R` — change the selected lines
- `✓ p` — put over the selection; the replaced text becomes the unnamed register
- `✓ P` — put over the selection; registers stay as they were
- `✓ J` — join the selected lines
- `✓ gJ` — join the selected lines as they are
- `✓ r<c>` — replace every selected character
- `✓ I` — insert at the selection's start (block: on every row)
- `✓ A` — append after the selection (block: on every row)
- `✓ o O` — go to the selection's other end
- `✓ *` — search forward for the selection
- `✓ #` — search backward for the selection
- `✓ ctrl-a` — add to the first number on each selected line
- `✓ ctrl-x` — subtract from the first number on each selected line
- `✓ ctrl-c` — toggle comments on selected lines
- `✓ S<c>` — wrap selection in pair
- `✓ i<a> a<a>` — objects select (vi[ works)
- `✓ space y` — yank selection → clipboard

## Search

- `✓ gn gN` — select the next / previous search match (cgn then . changes each)
- `✓ / ?` — literal search forward / backward (live as you type or delete)
- `✓ n` — next match
- `✓ N` — previous match
- `✓ * #` — word under cursor, forward / backward
- `✓ g* g#` — word under cursor, also inside longer words
- `✓ :noh` — clear the persistent search highlight

## Leader (space)

- `✓ space |` — pipe line/selection through shell (:! runs)
- `✓ space e` — this file's folder, cursor on the file
- `✓ space f` — file finder
- `✓ space o` — open remote destination
- `✓ space t` — terminal in a split below (TERMINAL mode)
- `✓ space T` — terminal in this pane (= :terminal)
- `✓ space b` — buffers (MRU)
- `✓ space /` — Search the project (or return to it)
- `✓ space R` — Search with replacement
- `✓ space ?` — help buffer
- `✓ space y` — yank motion → system clipboard
- `✓ space p` — paste clipboard after
- `✓ space P` — paste clipboard before
- `✓ space d` — diagnostics picker
- `✓ space k` — hover docs
- `✓ space a` — actions here (fixes and code; in a folder: search, terminal, copy path)
- `✓ space s` — document symbols picker
- `✓ space S` — workspace symbols picker
- `✓ space j` — jumplist picker
- `✓ space u` — undo-tree browser
- `✓ space c` — cursor on next line too (multicursor)

## Panes

- `✓ :vs :sp` — split vertical / horizontal
- `✓ space w h / l / j / k / w / 1 / 2 / 3 / 4 / 5 / 6 / 7 / 8 / 9` — pane move / cycle / focus 1-9
- `✓ ctrl-o / ctrl-i` — jump back / forward (jumplist; tab is ctrl-i)
- `✓ space w v / -` — pane split (vertical / horizontal)
- `✓ space w q` — close pane (last → buffer)
- `✓ ctrl-w h / j / k / l / w / W / p` — window: left / down / up / right / next / previous / last used (ctrl-w ctrl-{key} is the same)
- `✓ ctrl-w s / v / n` — window: split below / split right / new empty buffer below
- `✓ ctrl-w c / q / o` — window: close / quit (:q) / keep only this one
- `✓ ctrl-w =` — window: equal sizes (panes always are)
- `✓ gt gT` — next / previous tab page; 3gt goes to tab page 3, 3gT three back
- `✓ g tab` — the tab page used before this one
- `✓ ctrl-w T` — window: move this pane to a new tab page
- `✓ ctrl-l` — redraw: full repaint when the terminal desyncs

## Git

- `✓ space g` — git…
- `✓ space g g` — status: changed files
- `✓ space g c` — commit (:wq commits, :q! aborts)
- `✓ space g b` — branches: switch / create
- `✓ space g l` — log
- `✓ space g h` — file history (visual: selected lines)
- `✓ space g B` — blame gutter / card
- `✓ space g s` — hunk: stage
- `✓ space g u` — hunk: unstage
- `✓ space g X` — hunk: discard
- `✓ space g p` — hunk: preview
- `✓ space g y` — permalink: copy
- `✓ space g o` — permalink: open
- `✓ s u -` — status: stage / unstage / toggle the file, hunk or section
- `✓ =` — status: show / hide the file's diff
- `✓ X` — status: discard (y confirms)
- `✓ cc ca` — status: commit / amend
- `✓ enter` — status: open the file at the cursor
- `✓ ]c [c` — status: next / prev hunk
- `✓ ]f [f` — next / prev file in commit diff
- `✓ enter` — open the commit that last changed this line (blame)
- `✓ q` — close this view (read-only buffers)

## Folder

- `✓ enter -` — open the file or folder under the cursor / the parent folder
- `✓ g.` — show or hide dotfiles
- `✓ cw dd o yy p` — rename / delete / new (name/ for a folder) / copy: edit the names, then :w
- `✓ :w :e!` — show the changes (y applies, q cancels) / drop them and list the folder again
- `✓ :filter :e` — narrow to matching names (:filter alone clears) / list the folder again

## Pickers

- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ up down left right tab s-tab` — picker navigation / arrows = hjkl everywhere
- `✓ ctrl-x` — Search: exclude/include match
- `✓ ctrl-space` — query field: qualifier, language and value suggestions
- `✓ ctrl-u` — query field: clear back to the start
- `✓ alt-c alt-w alt-r` — Search: match case / whole word / regex
- `✓ alt-h alt-i` — Files, Search: hidden files / ignored files
- `✓ ctrl-d` — Search: exclude/include file
- `✓ ctrl-r` — input field: insert register {reg} (" is the last yank/delete)
- `✓ ctrl-t` — Search: toggle replacement
- `✓ enter` — Search: open the result; in With: open the review
- `✓ ctrl-o` — Search: collect included matches

## Terminal

- `✓ space t` — terminal in a split below (TERMINAL mode)
- `✓ space T` — terminal in this pane (= :terminal)
- `✓ :terminal [cmd] :term` — new shell (or cmd) in this pane; space t opens one in a split below
- `✓ ctrl-\ ctrl-n` — TERMINAL → NORMAL: a live buffer that follows output while the cursor is on the last line; k stops, G resumes
- `✓ i I a A` — NORMAL → TERMINAL at the child's cursor; every key goes to the child, Esc and ctrl-w included
- `✓ :q :bd!` — :q hides a running terminal (space b returns), :bd! stops it; :qa refuses while terminals run, :qa! stops them
- `✓ :r !cmd` — insert a command's output below the cursor

## Command line

- `✓ :` — the command line (from Visual it opens with '<,'>)
- `✓ :N :N,M :. :$ :% :'a :'<,'> :/pat/ :?pat? :.+N :N;+M` — ranges: lines, marks, searches and offsets; :N alone goes to the line
- `✓ enter ctrl-j ctrl-m` — run the line (a failed line says so in error ink)
- `✓ esc ctrl-[ ctrl-c` — cancel; the view returns to where it was
- `✓ backspace ctrl-h` — delete before the cursor; on an empty line, cancel
- `✓ delete` — delete under the cursor (at the end: before it)
- `✓ ctrl-w` — delete the word before the cursor
- `✓ ctrl-u` — delete everything before the cursor
- `✓ left` — one character left
- `✓ right` — one character right
- `✓ ctrl-b home` — start of the line
- `✓ ctrl-e end` — end of the line; with a completion selected: put back what was typed
- `✓ ctrl-r` — insert a register (" a-z 0 : / +); ctrl-r ctrl-w / ctrl-a: the word / WORD under the cursor
- `✓ tab` — next completion, written into the line (after the last: what was typed)
- `✓ s-tab` — previous completion
- `✓ ctrl-p` — older history line; previous completion while the menu is open
- `✓ ctrl-n` — newer history line; next completion while the menu is open
- `✓ up` — older history line that starts with what is typed; the row above once one is selected
- `✓ down` — newer history line that starts with what is typed; the row below once one is selected
- `✓ ctrl-y` — keep the selected completion, close the menu
- `✓ ctrl-d` — list the completions
- `✓ ctrl-f` — edit this line in the command-line window
- `✓ ctrl-l` — repaint the screen
- `✓ q: q/ q?` — the command-line window: the : (or / ?) history as a buffer, the line being typed last
- `✓ enter` — in the command-line window: run the cursor's line (Insert too)
- `✓ ctrl-c :q` — in the command-line window: close it

## Command line: text

- `✓ :s/pat/new/[flags]` — replace matches, with a live preview (flags g i I c n &)
- `✓ :&[&]` — repeat the last :s (:&& keeps its flags)
- `✓ :~` — repeat the last :s with the last search pattern
- `✓ :g[!]/pat/cmd` — run cmd on every matching line (! on the others)
- `✓ :v/pat/cmd` — run cmd on every line that does not match
- `✓ :norm[al] {keys}` — type Normal-mode keys on each line of the range
- `✓ :d [x] [count]` — delete lines (into register x)
- `✓ :y [x] [count]` — yank lines (into register x)
- `✓ :m {address}` — move lines below the address
- `✓ :t {address}` — copy lines below the address (also :copy)
- `✓ :j[!] [count]` — join lines (! keeps the white space)
- `✓ :pu[!] [x]` — put register x below the line (! above)
- `✓ :> [count]` — indent lines (:>> twice)
- `✓ :< [count]` — unindent lines (:<< twice)
- `✓ :sor[t][!] [u][n][i]` — sort lines (! reversed)
- `✓ :[range]!cmd` — run a shell command; with a range, filter the lines through it
- `✓ :@: :@x` — repeat the last command line / run register x as commands
- `✓ :r !cmd` — insert the command's output below the cursor
- `✓ :u[ndo]` — undo one change
- `✓ :red[o]` — redo one undone change
- `✓ :k {a}` — set mark a on the line (also :mark)
- `✓ :marks` — list the marks
- `✓ :delm[arks][!] {a b c-e}` — delete marks (! every lowercase mark)
- `✓ :reg [names]` — list the registers (also :display)
- `✓ :noh[lsearch]` — clear the persistent search highlight

## Command line: buffers

- `✓ :b {number|name}` — go to a buffer by number or unique name fragment
- `✓ :bn[ext]` — go to the next buffer
- `✓ :bp[revious]` — go to the previous buffer (also :bNext)
- `✓ :bd[elete][!] [number]` — delete a buffer (! discards changes, stops a terminal)
- `✓ :ls[!]` — list the buffers (also :buffers :files)

## Command line: files and windows

- `✓ :w[!] [file]` — write the buffer (! over a file changed on disk)
- `✓ :wa[ll][!]` — write every modified buffer
- `✓ :wq[!] [file]` — write and quit
- `✓ :wqa[ll][!]` — write every modified buffer and quit (also :xall)
- `✓ :x[it][!] [file]` — write if modified, then quit (also :exit)
- `✓ :up[date][!] [file]` — write if modified
- `✓ :q[uit][!]` — close the window; the tab page when it is the last window in it; the buffer when it is the last window
- `✓ :qa[ll][!]` — quit all (fails on unsaved; ! discards) (also :quitall)
- `✓ :clo[se]` — close the window (never the last one)
- `✓ :on[ly]` — close every other window
- `✓ :e[dit][!] [file]` — edit a file, or reload this one
- `✓ :ene[w]` — edit a new, unnamed buffer
- `✓ :new` — split with a new, unnamed buffer
- `✓ :vne[w]` — split vertically with a new, unnamed buffer
- `✓ :vie[w] {file}` — open a file readonly
- `✓ :sp[lit] [file]` — split below
- `✓ :vs[plit] [file]` — split to the right
- `✓ :cd [dir|-]` — change the project root (- goes back)
- `✓ :pw[d]` — show the project root (Files, Search, terminals)
- `✓ :se[t] {ro|noro}` — make this buffer readonly, or writable again
- `✓ :h[elp] [topic]` — the help buffer (this text; / searches it)
- `✓ :mes[sages]` — the last 50 echo-row messages
- `✓ :ju[mps]` — the jumplist picker

## Command line: tab pages

- `✓ :tabn[ext] [N]` — go to the next tab page, or to tab page N
- `✓ :tabp[revious] [N]` — go to the previous tab page, or N back (also :tabNext)
- `✓ :tabfir[st]` — go to the first tab page (also :tabrewind)
- `✓ :tabl[ast]` — go to the last tab page
- `✓ :tabnew [file]` — open a new tab page after this one (also :tabedit)
- `✓ :tab {split|new}` — open the split in a new tab page; :tab split shows this buffer alone
- `✓ :tabc[lose][!] [N]` — close this tab page, or tab page N; its buffers stay open
- `✓ :tabo[nly][!]` — close every other tab page
- `✓ :tabm[ove] [N]` — move this tab page after tab page N (0 first, $ or nothing last, +N -N relative)

## Command line: strop

- `✓ :ter[minal] [cmd]` — an interactive shell in the selected namespace
- `✓ :terminal-local [cmd]` — a local shell, even from an SSH context
- `✓ :theme [name]` — choose a built-in interface palette
- `✓ :filter [text]` — narrow the folder to matching names; alone it clears
- `✓ :remote [root|home|connect|disconnect|clear|list]` — choose a remote destination, or manage connections
- `✓ :trust` — authorize the pending endpoint or project command
- `✓ :explain` — why: workspaces, LSP readiness/failures, effective config
- `✓ :format` — format the buffer through the language server
- `✓ :rename {new}` — rename the symbol under the cursor
- `✓ :symbols` — the document symbols picker
- `✓ :cope[n]` — show the collection, on its current hit (the quickfix list) (also :cwindow)
- `✓ :cn[ext]` — next hit of the collection
- `✓ :cp[revious]` — previous hit of the collection (also :cNext)
- `✓ :cc [N]` — hit N of the collection
- `✓ :cdo {cmd}` — run a command on every hit line of the collection
- `✓ :cfdo {cmd}` — run a command once in every file of the collection
- `✓ :select-next` — select the next occurrence of the word or selection
- `✓ :select-all` — select every occurrence in the buffer
- `✓ :select-skip` — skip the next occurrence candidate
- `✓ :select-pop` — drop the last-added occurrence
- `✓ :tab-size [N|auto]` — indentation width for this file (alone: a selector)
- `✓ :indent-style [spaces|tabs|auto]` — indent with tabs or spaces in this file
- `✓ :recover [list|restore N|discard N]` — recover unsaved text after a crash

## Missing

Neovim binds these; in strop the key does nothing. An operator on this list
(`=`, `gq`, `gw`, `zf`) still takes its motion, so the motion's keys never run as
commands.

- Normal: `zfj` `zo` `zc` `za` `zR` `zM` `zj` `zk` `zd` — folds
- Normal: `U` — undo every change on the line
- Normal: `==` `=j` `=ip` — the reindent operator
- Normal: `gqq` `gqip` `gwip` — reflow text to the text width
- Normal: `dis` `das` — the sentence text objects
- Normal: `ga` `g8` — show the character under the cursor as a code
- Normal: `gx` `gf` `gF` — open the URL or file name under the cursor
- Normal: `gR` — virtual replace mode
- Normal: `K` — look the word under the cursor up (hover is `space k`)
- Normal: `][` `[]` `]m` `[m` — section-end and method motions
- Normal: `z=` `zg` `]s` `[s` — spelling
- Normal: `gm` `gM` `go` — middle of the line, byte offset
- Normal: `zh` `zl` `zH` `zL` `ze` `zs` — horizontal scrolling
- Normal: `do` `dp` — diff mode
- Normal: `ctrl-w x` `ctrl-w r` `ctrl-w H` `ctrl-w J` `ctrl-w K` `ctrl-w L` `ctrl-w +` `ctrl-w -` `ctrl-w _` `ctrl-w |` — move, rotate and resize windows (panes are always equal)
- Visual: `=` `gq` `gw` `zf` — reindent, reflow and fold the selection
- Insert: `ctrl-v` `ctrl-k` `ctrl-a` — insert a literal key, a digraph, the last inserted text
- Ex: `:fold` `:mksession` `:digraphs` — folds, session files, digraphs
- Ex: `:tabs` `:tabdo` `:tabfind` `:tcd` — list tab pages, run a command in each, find into one, a directory per tab page

## Reassigned

Neovim binds these; strop uses the key for something else.

- Normal: `space` — Neovim: move right. strop: the leader.
- Normal: `Q` — Neovim: replay the last recorded register. strop: toggle a cursor here (`@@` replays).
- Normal: `gd gD` — Neovim: go to the local or global declaration by search. strop: language-server definition and declaration.
- Normal: `gI` — Neovim: insert in column 1. strop: language-server implementation.
- Normal: `gr` — Neovim: replace with a virtual character (`gr{char}`). strop: only the prefix of the language-server keys `grr` `grn` `gra` `gri`.
- Normal: `gs` — Neovim: sleep. strop: switch between source and header.
- Normal: `ds cs ys` — Neovim: nothing (`s` is not a motion). strop: delete, change, add a surrounding pair.
- Visual: `S` — Neovim: change the selected lines. strop: wrap the selection in a pair (`C` and `R` change lines).
- Visual: `ctrl-c` — Neovim: leave Visual. strop: toggle comments on the selected lines (`Esc` leaves).
- Insert: `ctrl-e ctrl-y` — Neovim: insert the character below or above. strop: dismiss or accept the completion menu.
