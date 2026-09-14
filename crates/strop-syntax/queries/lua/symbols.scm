; 0063 §2 syntax fallback. `function a:b()` names use
; method_index_expression and classify as methods in Rust code.
(function_declaration name: (_) @name) @function.decl
