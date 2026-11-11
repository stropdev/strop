# strop in half a minute: the everyday loop, on the real binary.
#
#   Space f      find a file by part of its name
#   ciw  n  .    change a word, repeat the change on the next match, :w
#   Space /      search the project
#   Space R      replace across files: Enter reviews the diff, Enter
#                again changes the files and writes them
#   Space g g    git status: s stages, cc opens the commit message,
#                :wq commits
#
# The project is demos/fixtures/proj; demos/scripts/fixture.sh copies it
# to /tmp/proj as a git repository before recording starts. The terminal
# theme is strop's default palette, hone (crates/strop-core/src/theme/
# hone.rs), so the shell around the editor matches it.
#
# Render (.github/workflows/demo.yml does the same), from the repository
# root with strop on PATH inside the VHS image:
#   vhs demos/demo.tape --output demo-artifacts/demo.gif
Output demos/demo.gif
Set FontSize 22
Set Width 1280
Set Height 720
Set Padding 24
Set Framerate 24
Set Theme { "name": "hone", "background": "#15171c", "foreground": "#e6e1d6", "cursor": "#e8a848", "selection": "#30405a", "black": "#101217", "red": "#e8707c", "green": "#8fc48a", "yellow": "#d9a85a", "blue": "#7fb2d4", "magenta": "#c99ce0", "cyan": "#8ecdb8", "white": "#cfcac0", "brightBlack": "#4a5160", "brightRed": "#f08a94", "brightGreen": "#a6d6a0", "brightYellow": "#e8a848", "brightBlue": "#9ccbe8", "brightMagenta": "#d9b2f0", "brightCyan": "#a8e0d0", "brightWhite": "#e6e1d6" }

Hide
Type "sh demos/scripts/fixture.sh && cd /tmp/proj && clear"
Enter
Sleep 1s
Show

Type "strop src/main.rs"
Enter
Sleep 1.2s

# Space f: part of a name is enough; Enter opens the top row
Space
Sleep 600ms
Type "f"
Sleep 1s
Type@150ms "stone"
Sleep 1.2s
Enter
Sleep 1.2s

# ciw changes a word; n finds the next one and . repeats the change
Type "/Coarse"
Sleep 500ms
Enter
Type "n"
Sleep 700ms
Type "ciw"
Sleep 500ms
Type@120ms "Fine"
Escape
Sleep 800ms
Type "n"
Sleep 600ms
Type "."
Sleep 1s
Type ":w"
Enter
Sleep 1s

# Space /: search the project; Enter opens the hit under the bar
Space
Type "/"
Sleep 800ms
Type@150ms "deburr"
Sleep 1.5s
Down
Sleep 700ms
Enter
Sleep 1.2s

# Space R: the same search, with the cursor in a With field. Enter there
# opens the review; Enter in the review changes the files and writes them.
Space
Type "R"
Sleep 1s
Type@150ms "burnish"
Sleep 1.6s
Enter
Sleep 2.2s
Enter
Sleep 1.6s

# Space g g: status. s stages the row under the cursor, cc commits.
Space
Type "g"
Sleep 400ms
Type "g"
Sleep 1.3s
Type "s"
Sleep 1.2s
# on the section heading, s stages every file under it
Type "k"
Sleep 500ms
Type "s"
Sleep 1.2s
Type "cc"
Sleep 1.2s
Type "i"
Type@60ms "Burnish between passes, finer stones"
Escape
Sleep 800ms
Type ":wq"
Sleep 500ms
Enter
Sleep 2.5s
