; 0107 syntax fallback. Node names follow the grammar's own
; queries/tags.scm. A `def` inside a class or module is a method.
(class name: [(constant) (scope_resolution)] @name) @class.decl
(module name: [(constant) (scope_resolution)] @name) @module.decl
(method name: (_) @name) @function.decl
(singleton_method name: (_) @name) @method.decl
