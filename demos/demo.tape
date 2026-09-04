# strop prototype feel-check (plan 0004 §4 tape 1, draft):
# f-labels, ci[ change, live search-motion preview, flash.
# Render: docker run --rm -v "$PWD:/vhs" -w /vhs --entrypoint sh \
#   ghcr.io/charmbracelet/vhs -c 'install -m755 dist/strop /usr/local/bin/strop && vhs demos/demo.tape'
Output demos/demo.gif
Set FontSize 24
Set Width 1400
Set Height 760
Set Theme "Catppuccin Mocha"

Type "cd /tmp"
Enter
Type "strop proj/demo.rs"
Enter
Sleep 1.2s

# f-labels: every candidate on the line lights up
Type "0f"
Sleep 1.2s
Type "["
Sleep 700ms

# ci[ — the change; flash marks what fell
Type "ci"
Sleep 900ms
Type "["
Sleep 700ms
Type "Cut"
Escape
Sleep 900ms

# d/Mirror — the range previews live as the pattern types
Type "0d/"
Sleep 800ms
Type@350ms "Mirror"
Sleep 1.4s
Enter
Sleep 1.2s

Type ":q!"
Enter
Sleep 500ms

# picker: which-key card, file finder, live grep with preview
Type "strop proj/demo.rs"
Enter
Sleep 1s
Space
Sleep 900ms
Type "f"
Sleep 1.4s
Type "demo"
Sleep 1s
Enter
Sleep 900ms
Space
Type "/"
Sleep 700ms
Type@250ms "deburr"
Sleep 1.6s
Enter
Sleep 1s

Type ":q!"
Enter
Sleep 400ms

# git working surface: gutter signs, hunk nav, hunk card, undo
Type "cd /tmp/proj && git init -q && git remote add origin git@github.com:stropdev/strop.git && git add . && git -c user.email=t@t.t -c user.name=t commit -qm init"
Enter
Sleep 1.6s
Type "sed -i 's/Mirror/MirrorPolish/' demo.rs && git -c user.email=t@t.t -c user.name=t commit -qam 'mirror polish finish'"
Enter
Sleep 1.6s
Type "strop demo.rs"
Enter
Sleep 1.2s
Type "Go"
Sleep 300ms
Type "fn strop() {}"
Escape
Sleep 1.2s
Type "gg"
Sleep 400ms
Type "]c"
Sleep 1s
Space
Sleep 300ms
Type "g"
Sleep 300ms
Type "p"
Sleep 1.6s
Type "q"
Sleep 400ms
Space
Type "g"
Type "u"
Sleep 1.2s

# commit browser: dive commit → files → delta view (0010 diff surfaces)
Space
Type "g"
Type "l"
Sleep 2s
Type "j"
Sleep 600ms
Enter
Sleep 1.4s
Type "jj"
Sleep 400ms
Enter
Sleep 1.6s
Type "q"
Sleep 400ms
Type "q"
Sleep 300ms
Type "q"
Sleep 500ms
Space
Type "g"
Type "b"
Sleep 2s
Type "x"
Sleep 400ms
Type "jjj"
Sleep 300ms
Space
Type "g"
Type "y"
Sleep 1.2s

# splits: :vs shares the buffer; C-w w cycles panes
Type ":vs"
Enter
Sleep 1.6s
Type "jjj"
Sleep 500ms
# C-w w cycles panes (C-w is a prefix, like vim)
Ctrl+w
Type "w"
Sleep 1
Ctrl+w
Type "w"
Sleep 1.2s
Type ":q"
Enter
Sleep 600ms

# :help — keybindings as a real buffer; / searches it
Space
Type "?"
Sleep 1.4s
Type "/undo-tree"
Sleep 400ms
Enter
Sleep 1.2s
Type "q"
Sleep 500ms

# lsp: rust-analyzer diagnostics, picker, hover
Escape
Ctrl+C
Sleep 600ms
Type "cd /tmp/lsp-proj"
Enter
Type "strop src/main.rs"
Enter
Sleep 14s
Space
Type "d"
Sleep 2s
Escape
Sleep 500ms
Type "jfx"
Sleep 300ms
Space
Type "k"
Sleep 3s
Escape
Sleep 500ms

Type ":q!"
Enter
Sleep 400ms
