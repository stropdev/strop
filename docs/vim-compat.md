# Vim compatibility

Generated from the command table (`cargo test` pins freshness; STROP_REGEN=1 rewrites).
`✓` ships exactly; `(soon)` is a planned slot.

## normal

- `✓ h j k l` — move (never off the line)
- `✓ w b e W B E` — word / WORD motions
- `✓ zz zt zb` — center/top/bottom the cursor line
- `✓ H M L` — top/middle/bottom visible line
- `✓ ZZ` — write + close window
- `✓ gv` — reselect last visual
- `✓ gi` — insert at last insert
- `✓ g;` — older change (changelist)
- `✓ g,` — newer change (changelist)
- `✓ ge gE { }` — word-end back / paragraph motions
- `✓ 0 $ G %` — line/file/pair jumps
- `✓ gg` — top of file
- `✓ enter` — line down, first non-blank (blame gutter: dive)
- `✓ tab` — jump list forward (ctrl-i)
- `✓ ctrl-r` — redo
- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ ctrl-^` — alternate buffer
- `✓ q<a>` — record macro into register
- `✓ @<a>` — play macro (count repeats)
- `✓ gr` — references (LSP)
- `✓ gI` — implementation (LSP)
- `✓ gy` — type definition (LSP)
- `✓ gD` — declaration (LSP)
- `✓ ]d [d` — next/prev diagnostic
- `✓ gd` — goto definition (LSP)
- `✓ gs` — switch source/header (clangd)
- `✓ f<c> F<c> t<c> T<c>` — find/till char (candidates light up)
- `✓ :` — ex command line
- `✓ / ?` — search forward / backward
- `✓ n` — next match
- `✓ N` — previous match
- `✓ ]c [c` — next / prev git hunk
- `✓ m<a>` — set mark at cursor
- `✓ '<a> `<a>` — jump to mark
- `✓ * #` — word under cursor, forward / backward
- `✓ ; ,` — repeat find, same / reversed
- `✓ |` — column motion (vim); pipe moved to space |
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
- `✓ ^` — first non-blank
- `✓ ~` — toggle case
- `✓ S` — change line
- `✓ u ctrl-r` — undo / redo (one unit per command)
- `✓ "+y "+p "+P` — system clipboard: yank / paste after / before
- `✓ "xy "xp` — named register: yank / paste
- `✓ v V` — visual / visual-line

## visual

- `✓ d y c x > <` — operate on selection
- `✓ S<c>` — wrap selection in pair
- `✓ i<a> a<a>` — objects select (vi[ works)
- `✓ space y` — yank selection → clipboard

## insert

- `✓ esc` — normal mode (session = one undo unit)
- `✓ backspace` — delete back
- `✓ enter` — new line (auto-indent)
- `✓ } ] )` — closer on indent-only line dedents

## leader

- `✓ space |` — pipe line/selection through shell (:! runs)
- `✓ space f` — file finder
- `✓ space b` — buffers (MRU)
- `✓ space /` — live grep
- `✓ space R` — global search & replace
- `✓ space ?` — this popup
- `✓ space y` — yank motion → system clipboard
- `✓ space p` — paste clipboard after
- `✓ space P` — paste clipboard before
- `✓ space d` — diagnostics picker
- `✓ space k` — hover docs
- `· space j` — jumplist picker (soon)
- `✓ space u` — undo-tree browser
- `✓ space c` — cursor on next line too (multicursor)

## git

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

## ex+panes

- `✓ :w :q :q! :wq :w {file}` — write / quit (force) / write-quit / write-as
- `✓ :[range]s/a/b/[g] :N :% :N,Md :N,My` — substitute (literal) / goto line / ranged delete+yank
- `✓ ctrl-d ctrl-u ctrl-f ctrl-b` — half/full page scroll (count = lines)
- `✓ :e` — edit file
- `✓ :help` — help buffer (this text — / searches it)
- `✓ :vs :sp` — split vertical / horizontal
- `✓ ctrl-w h / l / j / k / w` — pane move / cycle
- `✓ ctrl-o / ctrl-i (tab)` — jump back / forward (jumplist)
- `✓ ctrl-w v / s` — pane split (vs / sp)
- `✓ :view / -R / :set ro,noro` — readonly browsing
- `✓ ctrl-w q` — close pane (last → buffer)
- `✓ up down left right tab s-tab` — picker navigation / arrows = hjkl everywhere
- `✓ ctrl-x` — replace picker: exclude/include match
