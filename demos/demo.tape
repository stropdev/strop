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
