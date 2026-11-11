# Vim compatibility

Generated from the command table (`strop compat`).
`✓` ships exactly; `(soon)` is a planned slot.

## Moving

- `✓ h j k l` — move (never off the line)
- `✓ w b e W B E` — word / WORD motions
- `✓ zz zt zb` — center/top/bottom the cursor line
- `✓ H M L` — top/middle/bottom visible line
- `✓ g;` — older change (changelist)
- `✓ g,` — newer change (changelist)
- `✓ ge gE { }` — word-end back / paragraph motions
- `✓ 0 $ G %` — line/file/pair jumps
- `✓ g<space>` — collection: open full source at caret
- `✓ gg` — top of file
- `✓ enter` — line down, first non-blank (blame gutter: dive)
- `✓ tab` — jump list forward (ctrl-i)
- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ ctrl-^` — alternate buffer
- `✓ gr` — references (LSP)
- `✓ gI` — implementation (LSP)
- `✓ gy` — type definition (LSP)
- `✓ gD` — declaration (LSP)
- `✓ ]d` — next diagnostic
- `✓ [d` — previous diagnostic
- `✓ gd` — goto definition (LSP)
- `✓ gs` — switch source/header (clangd)
- `✓ f<c> F<c> t<c> T<c>` — find/till char (candidates light up)
- `✓ ]f` — next file card (collections)
- `✓ [f` — previous file card (collections)
- `✓ ]e` — next excerpt (collections; +/- context)
- `✓ [e` — previous excerpt (collections; +/- context)
- `✓ ]c` — next git hunk
- `✓ [c` — previous git hunk
- `✓ m<a>` — set mark at cursor
- `✓ '<a> `<a>` — jump to mark
- `✓ ; ,` — repeat find, same / reversed
- `✓ |` — column motion (vim); pipe moved to space |
- `✓ ^ _` — first non-blank

## Editing

- `✓ ZZ` — write + close window
- `✓ gi` — insert at last insert
- `✓ gb` — select next occurrence of word/selection
- `✓ gB` — select all occurrences in buffer
- `✓ ctrl-r` — redo
- `✓ q<a>` — record macro into register
- `✓ @<a>` — play macro (count repeats)
- `✓ Q` — toggle cursor at point (multicursor)
- `✓ d y c > <` — operators + motion/object (live preview)
- `✓ dd yy cc` — line delete/yank/change
- `✓ D` — delete to line end
- `✓ C` — change to line end
- `✓ Y` — yank line
- `✓ s` — substitute char
- `✓ x X` — delete char / char back
- `✓ iw i" i' i( i[ i{` — inner objects (quotes scan the line)
- `✓ ds" cs"' ysiw"` — surround: delete / change / add
- `✓ i a A o O I` — insert (auto-indent)
- `✓ p P` — paste after / before
- `✓ r<c>` — replace char
- `✓ J .` — join lines · repeat change
- `✓ ~` — toggle case
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
- `✓ v V` — visual / visual-line
- `✓ d y c x > <` — operate on selection
- `✓ ctrl-c` — toggle comments on selected lines
- `✓ S<c>` — wrap selection in pair
- `✓ i<a> a<a>` — objects select (vi[ works)
- `✓ space y` — yank selection → clipboard

## Search

- `✓ / ?` — literal search forward / backward (live as you type or delete)
- `✓ n` — next match
- `✓ N` — previous match
- `✓ * #` — word under cursor, forward / backward
- `✓ :noh` — clear the persistent search highlight

## Leader (space)

- `✓ space |` — pipe line/selection through shell (:! runs)
- `✓ space e` — reveal source in its Directory buffer
- `✓ space f` — file finder
- `✓ space o` — open remote destination
- `✓ space t` — terminal in a split below (TERMINAL mode)
- `✓ space T` — terminal in this pane (= :terminal)
- `✓ space b` — buffers (MRU)
- `✓ space /` — Search workspace (resume investigation)
- `✓ space R` — Search with replacement
- `✓ space ?` — help buffer
- `✓ space y` — yank motion → system clipboard
- `✓ space p` — paste clipboard after
- `✓ space P` — paste clipboard before
- `✓ space d` — diagnostics picker
- `✓ space k` — hover docs
- `✓ space a` — actions here (fixes, code, files)
- `✓ space s` — document symbols picker
- `✓ space S` — workspace symbols picker
- `✓ space j` — jumplist picker
- `✓ space u` — undo-tree browser
- `✓ space c` — cursor on next line too (multicursor)

## Panes

- `✓ :vs :sp` — split vertical / horizontal
- `✓ space w h / l / j / k / w / 1 / 2 / 3 / 4 / 5 / 6 / 7 / 8 / 9` — pane move / cycle / focus 1-9
- `✓ ctrl-o / ctrl-i (tab)` — jump back / forward (jumplist)
- `✓ space w v / -` — pane split (vertical / horizontal)
- `✓ space w q` — close pane (last → buffer)
- `✓ ctrl-w h ctrl-w j ctrl-w k ctrl-w l ctrl-w w ctrl-w W ctrl-w p` — window: left / down / up / right / next / previous / last used (ctrl-w ctrl-{key} is the same)
- `✓ ctrl-w s ctrl-w v ctrl-w n` — window: split below / split right / new empty buffer below
- `✓ ctrl-w c ctrl-w q ctrl-w o` — window: close / quit (:q) / keep only this one
- `✓ ctrl-w =` — window: equal sizes (panes always are)
- `✓ ctrl-l` — redraw: full repaint when the terminal desyncs

## Git

- `✓ space g` — git…
- `✓ space g l` — commit browser
- `✓ space g h` — file history (visual: selected lines)
- `✓ space g b` — toggle blame gutter / card
- `✓ space g y` — permalink: copy
- `✓ space g o` — permalink: open
- `✓ space g u` — hunk: undo unstaged (restore from index)
- `✓ space g s` — hunk: stage (live→index)
- `✓ space g S` — hunk: unstage (index→HEAD)
- `✓ space g p` — hunk: preview
- `✓ ]f [f` — next / prev file in commit diff
- `✓ enter` — dive into the line's commit (blame gutter)
- `✓ q` — close surface (readonly buffers)

## Pickers

- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ up down left right tab s-tab` — picker navigation / arrows = hjkl everywhere
- `✓ ctrl-x` — Search: exclude/include match
- `✓ ctrl-space` — query field: qualifier, language and value suggestions
- `✓ ctrl-d` — Search: exclude/include file
- `✓ ctrl-r` — input field: insert register {reg} (" is the last yank/delete)
- `✓ ctrl-t` — Search: toggle replacement
- `✓ enter` — Search Find: open source; With: prepare review
- `✓ ctrl-o` — Search: collect included matches

## Terminal

- `✓ space t` — terminal in a split below (TERMINAL mode)
- `✓ space T` — terminal in this pane (= :terminal)
- `✓ :terminal [cmd] :term` — new shell (or cmd) in this pane; space t opens one in a split below
- `✓ ctrl-\ ctrl-n` — TERMINAL → NORMAL: a live buffer that follows output while the cursor is on the last line; k stops, G resumes
- `✓ i I a A` — NORMAL → TERMINAL at the child's cursor; every key goes to the child, Esc and ctrl-w included
- `✓ :q :bd!` — :q hides a running terminal (space b returns), :bd! stops it; :qa refuses while terminals run, :qa! stops them
- `✓ :r !cmd` — insert a command's output below the cursor

## Ex commands

- `✓ :` — ex command line
- `✓ :w :q :q! :wq :w {file} :trust` — write / quit (force) / write-quit / write-as
- `✓ :[range]s/a/b/[g] :N :% :N,Md :N,My` — substitute (buffer-search grammar + live preview) / goto line / ranged delete+yank
- `✓ :e` — edit file
- `✓ :help` — help buffer (this text — / searches it)
- `✓ :theme` — choose a built-in interface palette
- `✓ :jumps :symbols` — jumplist / document symbols picker
- `✓ :format :rename` — format buffer / rename symbol: :rename NEW
- `✓ :recover` — draft recovery: list, restore N, discard N, consent remote|off
- `✓ :qa :qall` — quit all (fails on unsaved; bang discards)
- `✓ :collection source|expand|contract` — source, expand or contract the excerpt at the caret
- `✓ :select-next :select-all :select-skip :select-pop` — occurrence selection: next / all / skip / pop last-added
- `✓ :view :set` — readonly browsing (:set ro/noro; CLI: -R)
- `✓ :tab-size :indent-style :search-options` — source indentation and search visibility controls
- `✓ :fs :browse :filter` — filesystem actions, namespace-aware browsing and folder filtering
- `✓ :apply-change :cancel-change :save-change` — review: apply text or filesystem changes, cancel, or save changed text
