; Adapted from Helix markdown-rustdoc/injections.scm (MPL-2.0).
; See ../LICENSE-HELIX-MPL. An explicit unknown label is never a Rust default.
(fenced_code_block
 . (fenced_code_block_delimiter)
 . (block_continuation)*
 . (code_fence_content) @injection.content
 (#set! injection.language "rust")
 (#set! injection.include-unnamed-children))

; Rustdoc attributes are inert syntax labels, never commands to execute.
(fenced_code_block
 (info_string (language) @_rustdoc)
 (code_fence_content) @injection.content
 (#match? @_rustdoc "^(rust|ignore|should_panic|no_run|compile_fail|standalone_crate|edition2015|edition2018|edition2021|edition2024)(,(rust|ignore|should_panic|no_run|compile_fail|standalone_crate|edition2015|edition2018|edition2021|edition2024))*$")
 (#set! injection.language "rust")
 (#set! injection.include-unnamed-children))
