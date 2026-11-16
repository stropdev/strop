; 0107 syntax fallback, for TypeScript and TSX. Node names follow the
; grammar's own queries/tags.scm and the JavaScript query it extends.
(class_declaration name: (type_identifier) @name) @class.decl
(abstract_class_declaration name: (type_identifier) @name) @class.decl
(interface_declaration name: (type_identifier) @name) @interface.decl
(enum_declaration name: (identifier) @name) @enum.decl
(internal_module name: (_) @name) @module.decl
(module name: (_) @name) @module.decl
(method_definition name: (_) @name) @method.decl
(method_signature name: (_) @name) @method.decl
(abstract_method_signature name: (_) @name) @method.decl
(function_declaration name: (identifier) @name) @function.decl
(generator_function_declaration name: (identifier) @name) @function.decl
(function_signature name: (identifier) @name) @function.decl
(variable_declarator
	name: (identifier) @name
	value: [(arrow_function) (function_expression)]) @function.decl
