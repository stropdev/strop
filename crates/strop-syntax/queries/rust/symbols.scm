; 0063 §2 syntax fallback: declaration extraction for `kind:` atoms
; and the workspace-symbols surface. Macro definitions are deliberately
; absent — syntax extraction does not guarantee macro-expanded or
; inferred declarations (plans/0063 §2).
(function_item name: (identifier) @name) @function.decl
(function_signature_item name: (identifier) @name) @function.decl
(struct_item name: (type_identifier) @name) @struct.decl
(enum_item name: (type_identifier) @name) @enum.decl
(trait_item name: (type_identifier) @name) @interface.decl
(mod_item name: (identifier) @name) @module.decl
(const_item name: (identifier) @name) @constant.decl
(static_item name: (identifier) @name) @constant.decl
