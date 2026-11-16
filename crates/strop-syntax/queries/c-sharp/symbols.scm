; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm.
(namespace_declaration name: (_) @name) @module.decl
(file_scoped_namespace_declaration name: (_) @name) @module.decl
(class_declaration name: (identifier) @name) @class.decl
(record_declaration name: (identifier) @name) @class.decl
(struct_declaration name: (identifier) @name) @struct.decl
(interface_declaration name: (identifier) @name) @interface.decl
(enum_declaration name: (identifier) @name) @enum.decl
(method_declaration name: (identifier) @name) @method.decl
(constructor_declaration name: (identifier) @name) @method.decl
(local_function_statement name: (identifier) @name) @function.decl
