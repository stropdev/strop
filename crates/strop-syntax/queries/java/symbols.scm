; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm.
(class_declaration name: (identifier) @name) @class.decl
(record_declaration name: (identifier) @name) @class.decl
(interface_declaration name: (identifier) @name) @interface.decl
(annotation_type_declaration name: (identifier) @name) @interface.decl
(enum_declaration name: (identifier) @name) @enum.decl
(method_declaration name: (identifier) @name) @method.decl
(constructor_declaration name: (identifier) @name) @method.decl
