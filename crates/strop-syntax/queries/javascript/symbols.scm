; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm: classes, methods, functions, and functions bound to
; a name.
(class_declaration name: (identifier) @name) @class.decl
(method_definition name: (_) @name) @method.decl
(function_declaration name: (identifier) @name) @function.decl
(generator_function_declaration name: (identifier) @name) @function.decl
(variable_declarator
	name: (identifier) @name
	value: [(arrow_function) (function_expression)]) @function.decl
