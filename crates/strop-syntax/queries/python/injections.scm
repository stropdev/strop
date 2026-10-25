; Only the initial string expression can be a Python docstring.
; The typed range policy additionally rejects bytes/fstrings and single quotes.
([
 (module . (comment)* . (expression_statement (string) @injection.content))
 (class_definition body: (block . (comment)* . (expression_statement (string) @injection.content)))
 (function_definition body: (block . (comment)* . (expression_statement (string) @injection.content)))
]
 (#set! injection.language "markdown")
 (#set! injection.documentation "python"))
