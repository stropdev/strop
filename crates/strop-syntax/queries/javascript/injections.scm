; Shared by JavaScript/TypeScript/TSX and C/C++: only documentation comments.
((comment) @injection.content
 (#match? @injection.content "^/\\*\\*[^*]")
 (#set! injection.language "markdown")
 (#set! injection.documentation "block"))
