; Adapted from Helix runtime/queries/rust/injections.scm (MPL-2.0).
; See ../LICENSE-HELIX-MPL. Documentation groups are deliberately not combined.
((line_comment doc: (doc_comment)) @injection.content
 (#set! injection.language "markdown-rustdoc")
 (#set! injection.documentation "rust-line"))

((block_comment doc: (doc_comment)) @injection.content
 (#set! injection.language "markdown-rustdoc")
 (#set! injection.documentation "block"))
