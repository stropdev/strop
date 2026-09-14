; 0063 §2 syntax fallback. Methods carry field_identifier names;
; template and macro-generated declarations are honestly absent.
(function_definition
	declarator: (function_declarator declarator: [(identifier) (qualified_identifier) (field_identifier)] @name)) @function.decl
(declaration
	declarator: (function_declarator declarator: [(identifier) (qualified_identifier) (field_identifier)] @name)) @function.decl
(class_specifier name: (type_identifier) @name body: (_)) @class.decl
(struct_specifier name: (type_identifier) @name body: (_)) @struct.decl
(enum_specifier name: (type_identifier) @name body: (_)) @enum.decl
