; Splicery — experimental syntax highlighting via the presentation
; grammar (Strop plan 0083). Maps onto the existing capture classes
; only; a color is not a permission, a match count, or proof of a valid
; recipe. Implemented against Splicery c696b6e (spec/splicery.ebnf).
;
; Choices this query owns (plan 0083 §5, one per ambiguity):
;  - the `.group` suffix on a handle reference is @property
;    (Attribute); the handle name itself is @variable
;  - the heredoc opener `<<'TAG'` and the closing tag line are both
;    @punctuation.special; the body between them is @string
;  - `rx"…"` / `hex"…"` / plain strings keep distinct capture heads
;    (@string.regexp / @string.special / @string) for future theme
;    work; all map to the String class today

; ---------------------------------------------------------------------
; Document heads and verbs
; ---------------------------------------------------------------------

[
  "edit"
  "create"
  "rename"
  "remove"
  "as"
  "bytes"
  "with"
  "to"
  "replace"
  "before"
  "after"
  "insert"
  "delete"
  "each"
  "in"
  "line_matches"
  "frozen"
  "expect"
  "within"
  "one"
  "between"
  "and"
  "range"
] @keyword

; ---------------------------------------------------------------------
; Contextual spellings — `text`/`line` are keywords only inside a
; `one` selector; an identifier spelled `text` stays a name.
; ---------------------------------------------------------------------

(one_selector
  "text" @keyword)

(one_selector
  "line" @keyword)

; Reserved piece heads — callable spellings reserved by the language
; (0070's `drop` precedent), not identifiers.
[
  "keep"
  "indent"
  "lines"
  "like"
] @keyword

; ---------------------------------------------------------------------
; Handles: the binder takes the parameter class; references take the
; variable class; the `.group` suffix is an attribute. The `@` marker
; itself is an operator token (handles are its only home).
; ---------------------------------------------------------------------

(binding_statement
  handle: (handle
    name: (name) @variable.parameter))

(iteration
  handle: (handle
    name: (name) @variable.parameter))

(reference
  handle: (handle
    name: (name) @variable))

(reference
  group: (name) @property)

"@" @operator

; ---------------------------------------------------------------------
; Literals and comments
; ---------------------------------------------------------------------

(string) @string

(hex_literal) @string.special

(regex) @string.regexp

(heredoc_body) @string

[
  (heredoc_start)
  (heredoc_end)
] @punctuation.special

(escape_sequence) @string.escape

(nat) @number

(comment) @comment

; ---------------------------------------------------------------------
; Operators and punctuation
; ---------------------------------------------------------------------

":=" @operator

[
  "{"
  "}"
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ","
  "."
] @punctuation.delimiter
