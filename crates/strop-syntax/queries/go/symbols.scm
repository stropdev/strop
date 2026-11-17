; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm. A method's receiver type is not its container in
; the tree, so a method stands alone.
(function_declaration name: (identifier) @name) @function.decl
(method_declaration name: (field_identifier) @name) @method.decl
(type_spec name: (type_identifier) @name type: (struct_type)) @struct.decl
(type_spec name: (type_identifier) @name type: (interface_type)) @interface.decl
(method_elem name: (field_identifier) @name) @method.decl
