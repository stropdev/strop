; 0063 §2 syntax fallback. Module-level assignments are not captured:
; Python constants are inferred, not declared — honest coverage.
(function_definition name: (identifier) @name) @function.decl
(class_definition name: (identifier) @name) @class.decl
