# Sovel presentation grammar — EBNF coverage

Grammar-rule-to-EBNF-family coverage for `grammar.js`, against Sovel
**0.1-draft.4** (`spec/grammar.ebnf` + `spec/LEXING-AND-PARSING.md` at
sovel-handover `eab2ce3`). This is a tolerant presentation grammar for
syntax highlighting (Strop plan 0070); acceptance here is NOT language
acceptance. Deliberate presentation-level deviations are listed at the end.

| EBNF production(s) | Grammar rule(s) | Corpus file |
|---|---|---|
| `source_file`, `item`, `visibility` | `source_file`, `_item` (owns the `{attribute} [pub]` prefix), `_item_body` | declarations.txt, all |
| `module_decl`, `import_decl`, `module_path` | `module_declaration`, `import_declaration`, `module_path` | declarations.txt |
| `attribute` (flag/list/value forms) | `attribute`, reusing call-site `argument`/`argument_list` | declarations.txt, foreign.txt |
| `struct_decl`, `field_declarations`, `field_decl` | `struct_declaration`, `field_declarations`, `field_declaration` | declarations.txt |
| `enum_decl`, `variant_decl` | `enum_declaration`, `variant_declaration` | declarations.txt |
| `newtype_decl`, `alias_decl`, `const_decl` | `newtype_declaration`, `type_alias`, `const_declaration` | declarations.txt |
| `effect_decl`, `origin_parameter_names` | `effect_declaration`, `origin_parameter` | declarations.txt |
| `trait_decl`, `trait_member` | `trait_declaration` (incl. leading `dyn`), `_trait_member` | traits.txt |
| `impl_decl`, `impl_member`, `delegation_decl` | `impl_declaration`, `_impl_member`, `delegation_declaration` | traits.txt |
| `trait_bounds`, `trait_type` | `trait_bounds`, `trait_type` | traits.txt, functions.txt |
| `extern_block`, `extern_member`, `extern_function`, `extern_parameters` | `extern_block`, `_extern_member`, `extern_function`, `extern_parameter` | foreign.txt |
| `function_decl`, `function_signature` | `function_declaration`, `function_signature` | functions.txt, all |
| `parameter_list`, `parameter`, `receiver` | `parameter_list`, `parameter`, `receiver` | functions.txt |
| `context_parameters`, `context_parameter` | `context_parameters`, `context_parameter` | functions.txt |
| `macro_decl` | `macro_declaration` | macros_quotes.txt |
| `generic_parameters`, `generic_parameter` | `generic_parameters`, `generic_parameter` (+ named `type_parameter`, `effect_parameter`, `generic_origin_parameter`, `const_parameter`) | functions.txt |
| `generic_arguments`, `generic_argument`, `constant_argument` | `generic_arguments`, `generic_argument`, `constant_argument` | functions.txt, expressions.txt |
| `where_clause`, `constraint` | `where_clause`, `constraint` | functions.txt, effects_origins.txt |
| `type`, `type_prefix`, `reference_type` | `_type` (origin clause binds innermost, spec decision 2), `_type_prefix`, `reference_type` | types.txt |
| `function_type`, `call_mode`, `function_type_parameters` | `function_type`, `call_mode`, `function_type_parameter` | functions.txt |
| `abstract_type`, `abstract_bound` | `abstract_type`, `_abstract_bound` | types.txt |
| `type_atom`, `type_list`, `type_path` | `type_atom`, `type_list`, `type_path` (+ named `unit_type`, `parenthesized_type`, `tuple_type`, `array_type`) | types.txt |
| `origin_clause`, `origin_bound`, `origin_members`, `origin_member`, `origin_path` | `origin_clause`, `origin_bound`, `origin_member`, `origin_tail`, `origin_path` | effects_origins.txt, types.txt |
| `effect_clause`, `effect_expression`, `effect_atom`, `effect_set`, `effect_members`, `row_tail`, `effect_label`, `effect_name` | same names (effect set wins over a body block after `!` via precedence) | effects_origins.txt, functions.txt |
| `block`, `statement` family | `block`, `_statement`, `let_statement`, `return_statement`, `break_statement`, `continue_statement`, `assignment_statement`, `expression_statement`, `_block_statement` | control_flow.txt, expressions.txt |
| `place`, `assignment_operator` | `place` (delegation target), `assignment_operator`; assignment LHS is `_expression` — see deviations | expressions.txt, traits.txt |
| `expression`, `lambda_expression`, `lambda_parameters` | `_expression`, `lambda_expression` (the `move` variant outranks unary `move`), `_lambda_body`, `lambda_parameter` | expressions.txt |
| `logical_or` … `multiplicative` ladder | `binary_expression` (per-operator precedence table, left assoc) | expressions.txt |
| `unary_expression`, `spawn`, `await`, `fail_expression` | `unary_expression`, `spawn_expression`, `fail_expression` | expressions.txt |
| `postfix_expression`, `postfix_suffix`, `context_arguments`, `argument_list`, `argument` | `call_expression`, `member_expression` (incl. tuple index and `.name::<>`), `index_expression`, `propagation_expression`, `context_arguments`, `argument_list`, `argument` | expressions.txt |
| `primary_expression`, `value_path`, `expression_list`, `array_expression` | `_primary_expression`, `value_path` (incl. `drop`, `self`, `Self`, `::<>`), `expression_list`, `array_expression` (+ `unit_expression`, `parenthesized_expression`, `tuple_expression`) | expressions.txt |
| `struct_expression`, `field_initializers` | `struct_expression` (loses control-head ties via negative dynamic precedence, spec decision 6), `field_initializer` | expressions.txt, control_flow.txt |
| `block_expression` family | `if_expression`, `_condition` (incl. `if let` / `while let`), `match_expression`, `match_arm` (guards), `while_expression`, `for_expression`, `loop_expression`, `try_expression`, `catch_clause`, `region_expression`, `tasks_expression`, `unsafe_expression`, `comptime_expression` | control_flow.txt |
| `pattern`, `atomic_pattern`, `binding_pattern`, `tuple_pattern`, `constructor_pattern`, `field_patterns`, `slice_pattern`, `pattern_list` | `_pattern`, `or_pattern`, `_atomic_pattern`, `wildcard_pattern`, `binding_pattern` (`ref mut`, `@`), `tuple_pattern`, `parenthesized_pattern`, `pattern_list`, `constructor_pattern` (payload-free form requires a qualified path, spec decision 7), `field_patterns`, `field_pattern`, `slice_pattern` | patterns.txt |
| `macro_call`, `item_macro_call`, `token_group`, `token_tree` | `macro_call`, `item_macro_call`, `token_group`, `token_tree`, `_macro_token` (opaque non-delimiter runs; strings/chars kept whole; comments are extras) | macros_quotes.txt |
| `quote_expression`, `syntax_splice`, `item_splice` | `quote_expression` (all six forms), `syntax_splice` (`$x`, `${...}`), `item_splice` | macros_quotes.txt |
| `literal`, `INTEGER`, `FLOAT`, `STRING`, `CHARACTER`, `IDENTIFIER`, keywords, comments | `literal`, `integer_literal`, `float_literal`, `string_literal`, `char_literal`, `boolean_literal`, `identifier`, `comment` | lexical.txt, expressions.txt |

## Lexical cases (LEXING-AND-PARSING.md terminals)

- `2.seconds()` / `2.0` / `1..3`: `float_literal` requires a digit after the
  point; tuple projection `.0` reuses `integer_literal`. Covered in
  expressions.txt.
- No `>>`/`<<` tokens: nested generic closings lex as separate `>` tokens.
  Covered in types.txt (`Vec<Vec<T>>`) and recovery.txt.
- `||` lambda versus Boolean OR: `lambda_expression` only begins an
  expression; grouped or-patterns in lambda parameters parse through
  `parenthesized_pattern`. Covered in expressions.txt.
- Non-nesting `/* … */` (spec defers nesting); `//`, `///`, `/** … */` all
  lex as one `comment` extra. Covered in lexical.txt.
- No raw strings, no interpolation, no lifetimes; escapes per spec.
- Incomplete input recovers locally (unfinished `#[derive(`, trailing
  `using`, `! {`, unclosed generics, quotes/splices, unterminated strings):
  recovery.txt asserts the tolerant shapes (with `ERROR`/`MISSING` nodes).

## Deliberate presentation-level deviations from the EBNF

- **Assignment targets** parse as `_expression` rather than `place`
  (`f() = 1;` is not rejected). This keeps statement-level recovery local
  and removes a GLR conflict class; target validity is the authoritative
  checker's job. `place` is retained for `delegate … to place;`.
- **Struct literals in control heads**: the spec excludes them; the grammar
  explores both readings and prefers the body block via negative dynamic
  precedence, so the spec-invalid form still gets a plausible tree
  (highlighting must not punish the user mid-edit).
- **Equality/comparison/range associate left** instead of not chaining —
  chains are semantic errors, not syntax shapes.
- Effect spellings (`read`, `write`, `alloc`, `io`, `foreign`, …) are plain
  identifiers; only `fail(T)`, `drop(T)`, `suspend`, `cancel`, `spawn` are
  spelled structurally in `effect_label`. The highlight query colors them
  contextually, never globally.
