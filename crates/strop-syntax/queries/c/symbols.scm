; 0063 §2 syntax fallback. Body presence separates definitions from
; type usages; `struct St;` forward declarations stay unmatched.
(function_definition
	declarator: (function_declarator declarator: (identifier) @name)) @function.decl
(declaration
	declarator: (function_declarator declarator: (identifier) @name)) @function.decl
(struct_specifier name: (type_identifier) @name body: (_)) @struct.decl
(enum_specifier name: (type_identifier) @name body: (_)) @enum.decl
