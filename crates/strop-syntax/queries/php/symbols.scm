; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm; a trait is an interface, as there.
(namespace_definition name: (namespace_name) @name) @module.decl
(class_declaration name: (name) @name) @class.decl
(interface_declaration name: (name) @name) @interface.decl
(trait_declaration name: (name) @name) @interface.decl
(enum_declaration name: (name) @name) @enum.decl
(method_declaration name: (name) @name) @method.decl
(function_definition name: (name) @name) @function.decl
