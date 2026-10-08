; Sovel — experimental syntax highlighting (Strop plan 0070).
; Hand-authored for the presentation grammar in
; crates/strop-syntax/grammars/sovel (Sovel 0.1-draft.4). Maps onto the
; existing capture classes only; a color is not a permission or a proof.
;
; Effect spellings (`read`, `write`, `alloc`, `io`, `foreign`, …) are NOT
; keywords anywhere in this file: they are plain identifiers that only get
; a class inside effect syntax (`effect_name`, `effect_atom`, `row_tail`).

; ---------------------------------------------------------------------
; Declarations and control flow
; ---------------------------------------------------------------------

[
  "module"
  "import"
  "as"
  "struct"
  "enum"
  "newtype"
  "type"
  "const"
  "effect"
  "trait"
  "dyn"
  "impl"
  "fn"
  "comptime"
  "let"
  "mut"
  "ref"
  "return"
  "break"
  "continue"
  "if"
  "else"
  "match"
  "while"
  "for"
  "in"
  "loop"
  "try"
  "catch"
  "fail"
  "region"
  "tasks"
  "on"
  "within"
  "unsafe"
  "extern"
  "pub"
  "macro"
  "quote"
  "move"
  "await"
  "once"
  "static"
  ; quote subforms (reserved keywords in 0.1-draft.4)
  "expr"
  "items"
  "pattern"
  ; `drop` is the reserved callable spelling, not an identifier
  "drop"
] @keyword

; Contract clause heads: capability authority, dependencies, effects,
; constraints — readable and textually distinct (0070 §5).
[
  "using"
  "from"
  "where"
  "effects"
  "origin"
  "origins"
  "excludes"
  "outlives"
  "disjoint"
  "delegate"
  "to"
] @keyword

; `spawn` starts an expression here but names an effect inside effect
; syntax, so it is captured contextually rather than globally.
(spawn_expression
  "spawn" @keyword)

; `spawn`/`suspend`/`cancel` name effects inside effect syntax.
(effect_name [
  "spawn"
  "suspend"
  "cancel"
] @type)

; ---------------------------------------------------------------------
; Names by declared role
; ---------------------------------------------------------------------

(function_signature
  name: (identifier) @function)

(macro_declaration
  name: (identifier) @function.macro)

(macro_call
  (module_path) @function.macro)

(extern_function
  name: (identifier) @function)

; Called names (plain paths and method calls; field access stays default).
(call_expression
  function: (value_path) @function)

(call_expression
  function: (member_expression
    member: (identifier) @function))

(struct_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(newtype_declaration
  name: (identifier) @type)

(type_alias
  name: (identifier) @type)

(trait_declaration
  name: (identifier) @type)

(effect_declaration
  name: (identifier) @type)

(const_declaration
  name: (identifier) @constant)

; Type positions. `type_path` only exists in type context, so the whole
; path takes @type; no uppercase guessing anywhere.
(type_path) @type

(value_path
  "Self" @type)

; Generic parameter declarations and effect/origin rows inside contracts.
(type_parameter) @type.parameter

(effect_parameter
  (identifier) @type.parameter)

(effect_atom
  (identifier) @type.parameter)

(effect_name
  (module_path
    (identifier) @type))

(row_tail
  (identifier) @type.parameter)

; Origin/capability binders and references.
(generic_origin_parameter
  (identifier) @variable.parameter)

(origin_parameter
  (identifier) @variable.parameter)

(origin_path
  (identifier) @variable.parameter)

(origin_tail
  (identifier) @variable.parameter)

(context_parameter
  name: (identifier) @variable.parameter)

(parameter
  (binding_pattern
    (identifier) @variable.parameter))

(extern_parameter
  name: (identifier) @variable.parameter)

(lambda_parameter
  (binding_pattern
    (identifier) @variable.parameter))

(region_expression
  name: (identifier) @variable)

(receiver
  "self" @variable.builtin)

(value_path
  "self" @variable.builtin)

(origin_path
  "self" @variable.builtin)

; The named foreign contract path stays a path, not a string.
(unsafe_expression
  contract: (module_path) @type)

; ---------------------------------------------------------------------
; Attributes: `#[path]`, `#[path(args)]`, `#[path = value]`
; ---------------------------------------------------------------------

(attribute
  (module_path) @attribute)

; ---------------------------------------------------------------------
; Literals and comments
; ---------------------------------------------------------------------

(string_literal) @string

(char_literal) @character

(integer_literal) @number

(float_literal) @float

(boolean_literal) @boolean

(comment) @comment

; ---------------------------------------------------------------------
; Operators and punctuation
; ---------------------------------------------------------------------

[
  "!"
  "?"
  "&"
  "&&"
  "||"
  "|"
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  ".."
  "..="
  "+"
  "-"
  "*"
  "/"
  "%"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "->"
  "=>"
  "@"
  ; quote splices `$x` / `${...}`
  "$"
] @operator

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
  ";"
  ":"
  "::"
  "."
] @punctuation.delimiter

"#" @punctuation.special
