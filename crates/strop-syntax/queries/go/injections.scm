; Adapted from Helix runtime/queries/go/injections.scm (MPL-2.0).
; See ../LICENSE-HELIX-MPL. Only declaration-attached block documentation.
(source_file
 (comment) @injection.content .
 [(package_clause) (type_declaration) (function_declaration)
  (method_declaration) (var_declaration) (const_declaration)]
 (#match? @injection.content "^/\\*")
 (#set! injection.language "markdown")
 (#set! injection.documentation "go-block"))
