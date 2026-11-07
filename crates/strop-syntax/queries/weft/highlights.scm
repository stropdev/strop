; Weft — syntax highlighting via the presentation grammar (Strop plan
; 0083). Maps onto the existing capture classes only; a color is not a
; proof of source validity. Weft revision a115f51, spec revision 3.

; ---------------------------------------------------------------------
; Header and declaration keywords
; ---------------------------------------------------------------------

[
  "review"
  "context"
  "at"
  "profile"
  "target"
  "proposal"
  "claim"
  "asset"
  "rule"
  "citation"
  "need"
  "assume"
  "check"
  "by"
] @keyword

; ---------------------------------------------------------------------
; Operators: propositional connectives and quantifiers are spelled
; keywords, but they operate; the spec's dual-role `in` covers both the
; quantifier binder and the membership operator with one token.
; ---------------------------------------------------------------------

[
  "and"
  "or"
  "not"
  "forall"
  "exists"
  "in"
] @keyword.operator

[
  "=="
  "!="
  "<="
  ":="
  "="
  "&"
  "|"
  "-"
  "..."
] @operator

; ---------------------------------------------------------------------
; Names by role: declarations, labels, binders and rule references.
; Plain value paths stay unstyled — no uppercase-means-type guessing.
; ---------------------------------------------------------------------

(review
  name: (ident) @variable)

(target_decl
  name: (ident) @variable)

(binding_decl
  name: (ident) @variable)

(set_binding
  name: (ident) @variable)

(need_decl
  name: (ident) @variable)

(assumption_decl
  label: (ident) @variable)

(check_decl
  label: (ident) @variable)

(rule_use
  rule: (ident) @variable)

(quantifier
  binder: (ident) @variable)

(predicate_call
  predicate: (ident) @function)

; ---------------------------------------------------------------------
; Literals and comments
; ---------------------------------------------------------------------

(integer) @number

(boolean) @constant.builtin

(string) @string

(escape_sequence) @string.escape

(comment) @comment

; ---------------------------------------------------------------------
; Punctuation
; ---------------------------------------------------------------------

[
  "{"
  "}"
  "("
  ")"
] @punctuation.bracket

[
  ";"
  ","
  "."
  ":"
] @punctuation.delimiter
