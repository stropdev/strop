# Weft grammar coverage

Grammar-rule ↔ EBNF-family map (spec `grammar.ebnf`, revision 3) with
the presentation deviations that keep editing stable. A green tree is
not source validity; `weft-frontend` remains the authority.

| EBNF family | Grammar rules | Notes |
|---|---|---|
| `source_file`/`review` | `source_file`, `review` | header order enforced (context/profile/target) |
| `scope_decl` | `scope_decl` | |
| `profile_decl` | `profile_decl` | |
| `target_decl` | `target_decl` | |
| `binding_decl`/`binding_kind` | `binding_decl` (`claim`/`asset`/`rule`) | |
| `set_binding` | `set_binding` | |
| `need_decl` | `need_decl` | |
| `assumption_decl` | `assumption_decl` | |
| `check_decl`/`rule_use` | `check_decl`, `rule_use` | |
| `or_expr` | `expression`, `boolean_or_expression` | left-associative |
| `and_expr` | `_and_expression`, `boolean_and_expression` | left-associative |
| `not_expr` | `_not_expression`, `not_expression` | right-nesting (`not not d`) |
| `atom` paren | `group_expression` | dynamic precedence over the bare-comparison reading |
| `atom` quantifier | `quantifier` | `forall`/`exists` |
| `atom` predicate_call | `predicate_call` | arguments are `value`s only, as upstream |
| `atom` comparison | `comparison` | ONE optional operator; chains parse as errors, as upstream rejects them |
| `comparison_operator` | `_comparison_operator` | `==` `!=` `in` `<=` |
| `operand`/`set_alt` | `set_expression`, `set_union_expression`, `set_difference_expression` | `\|`/`-` equal precedence, left |
| `set_inter` | `_set_inter`, `set_intersection_expression` | `&` tighter |
| `set_operand` | `_set_operand`, `parenthesized_set` | |
| `set_literal`/`set_items`/`set_item` | `set_literal`, `set_item` | trailing comma allowed |
| `...` spread | `spread` | maximal munch over `.` |
| `value`/`path` | `value`, `path` | |
| `IDENT` | `ident` | ASCII only, keyword boundary via `word` |
| `STRING` | `string`, `escape_sequence` | see deviations |
| `INTEGER` | `integer` | `0` or `[1-9][0-9]*`, no sign |
| `BOOLEAN` | `boolean` | |
| comments | `comment` | `//` to end of line |
| whitespace | `extras` | space/tab/LF/CRLF |

## Documented presentation deviations

- **Escape superset**: `escape_sequence` accepts `\<any non-newline>`;
  the spec allows only `\" \\ \n \r \t`. Keeps half-typed strings
  lexable while editing.
- **Optional closing quote**: an unterminated `"` colors as a string to
  end of line instead of an error cascade; upstream refuses raw
  newlines in strings.
- **Isolated CR is whitespace** here; the frontend lexer rejects it.
- **Reserved-word tolerance**: identifiers are never checked against the
  keyword list beyond tokenization; `check`-spelled idents parse as
  keywords exactly where the EBNF puts them, and upstream owns any
  residual refusal.
- **`(a)` in proposition position** derives twice (grouped bare
  comparison vs one-operand comparison over a parenthesized set). GLR
  resolves toward `group_expression`; both shapes are syntactically
  legal upstream (a bare operand there is a typing error, never a parse
  error), so the choice is presentational only.
