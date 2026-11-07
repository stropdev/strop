// tree-sitter-weft — presentation grammar for Weft (spec revision 3).
//
// Provenance: hand-authored against the Weft checkout
//   repository: /home/tarek/workspace/weft (MIT licensed)
//   revision:   a115f51 ("Ship pinned writfold.agent/1 control client and
//               local reuse gate"), spec revision 3
//   sources:    spec/grammar.ebnf, spec/language.md §Lexical,
//               crates/weft-frontend/src/lexer/token.rs (lexical oracle)
//   generator:  tree-sitter-cli 0.25.10, `tree-sitter generate --abi 15`
//   runtime:    tree-sitter 0.25.10 (ABI 13–15)
//
// This is a *presentation* grammar for Strop's syntax highlighting (Strop
// plan 0083). A colored or error-free tree is NOT proof of source validity;
// weft-frontend's lexer/parser remains the authoritative frontend.
//
// Documented disambiguation policies (spec/language.md §"Precedence and
// ambiguity decisions"):
//  - Comparisons do not chain: `comparison` holds one optional operator, so
//    `a == b == c` parses the third operand as an error, matching the
//    upstream rejection instead of silently associating.
//  - One operand grammar (`_set_alt`) serves scalar values and set
//    expressions; `(a)` in proposition position derives twice — a grouped
//    bare comparison (`group_expression`) or a one-operand comparison over
//    a `parenthesized_set`. GLR explores both; the dynamic precedence on
//    `group_expression` picks the grouping shape when both survive (a bare
//    operand in proposition position is an upstream typing error, never a
//    truth value, so the grouping shape is the honest presentation), while
//    a following comparison operator (`(a) == b`) prunes the group branch.
//  - A bare `true`/`false` is the atom itself (the EBNF lists BOOLEAN in
//    atom), not a one-operand comparison; a static precedence breaks that
//    reduce/reduce tie against `value`.
//  - `in` is one keyword token serving the quantifier binder and the
//    membership operator; grammar position decides the node.
//  - INTEGER keeps the spec shape `0 | [1-9][0-9]*`; a leading zero lexes
//    as two integers and shows as a syntax error, as upstream refuses it.
//  - Escape sequences accept any non-newline after `\` (spec allows only
//    \" \\ \n \r \t; the superset keeps half-typed strings editable) —
//    deviation recorded in COVERAGE.md.
//  - Isolated CR is whitespace here (upstream rejects it) — deviation
//    recorded in COVERAGE.md.
//  - No external scanner: strings are single-line and comments are
//    line-based, so regex tokens suffice.

module.exports = grammar({
  name: 'weft',

  word: $ => $.ident,

  extras: $ => [/[\t\n\r ]/, $.comment],

  conflicts: $ => [
    [$.comparison, $.set_expression],
  ],

  rules: {
    source_file: $ => $.review,

    review: $ => seq(
      'review',
      field('name', $.ident),
      '{',
      $.scope_decl,
      $.profile_decl,
      $.target_decl,
      repeat($.statement),
      '}',
    ),

    scope_decl: $ => seq(
      'context',
      field('context', $.string),
      'at',
      field('snapshot', $.string),
      ';',
    ),

    profile_decl: $ => seq('profile', field('profile', $.string), ';'),

    target_decl: $ => seq(
      'target',
      field('name', $.ident),
      '=',
      'proposal',
      field('proposal', $.string),
      ';',
    ),

    statement: $ => choice(
      $.binding_decl,
      $.set_binding,
      $.need_decl,
      $.assumption_decl,
      $.check_decl,
    ),

    binding_decl: $ => seq(
      field('kind', $._binding_kind),
      field('name', $.ident),
      '=',
      'citation',
      field('citation', $.string),
      ';',
    ),

    _binding_kind: $ => choice('claim', 'asset', 'rule'),

    set_binding: $ => seq(
      field('name', $.ident),
      ':=',
      field('value', $.set_expression),
      ';',
    ),

    need_decl: $ => seq('need', field('name', $.ident), ';'),

    assumption_decl: $ => seq(
      'assume',
      field('label', $.ident),
      ':',
      field('value', $.expression),
      ';',
    ),

    check_decl: $ => seq(
      'check',
      field('label', $.ident),
      ':',
      field('value', $.expression),
      optional($.rule_use),
      ';',
    ),

    rule_use: $ => seq('by', sep1(field('rule', $.ident), ',')),

    // ------------------------------------------------------------------
    // Propositions. `or` < `and` < `not` (right-nesting) < atoms; the
    // comparison holds ONE optional operator because comparisons never
    // chain (spec decision 1).
    // ------------------------------------------------------------------

    // Binary levels are visible nodes so the corpus locks associativity
    // and grouping, not just token order.
    expression: $ => choice($.boolean_or_expression, $._and_expression),

    boolean_or_expression: $ => prec.left(seq(
      field('left', $.expression),
      'or',
      field('right', $.expression),
    )),

    _and_expression: $ => choice($.boolean_and_expression, $._not_expression),

    boolean_and_expression: $ => prec.left(seq(
      field('left', $._and_expression),
      'and',
      field('right', $._and_expression),
    )),

    _not_expression: $ => choice($.not_expression, $._proposition_atom),

    not_expression: $ => prec.right(seq('not', $._not_expression)),

    _proposition_atom: $ => choice(
      $.group_expression,
      $.quantifier,
      $.predicate_call,
      $.comparison,
      prec(1, $.boolean),
    ),

    group_expression: $ => prec.dynamic(1, seq('(', $.expression, ')')),

    comparison: $ => seq(
      field('left', $._set_inter),
      optional(seq(
        field('operator', $._comparison_operator),
        field('right', $._set_inter),
      )),
    ),

    _comparison_operator: $ => choice('==', '!=', 'in', '<='),

    quantifier: $ => seq(
      field('kind', choice('forall', 'exists')),
      field('binder', $.ident),
      'in',
      field('domain', $.set_expression),
      '{',
      field('body', $.expression),
      '}',
    ),

    predicate_call: $ => seq(
      field('predicate', $.ident),
      '(',
      optional(sep1($.value, ',')),
      ')',
    ),

    // ------------------------------------------------------------------
    // One operand grammar for scalars and sets: `&` binds tighter than
    // the equal-precedence left-associative `|`/`-`.
    // ------------------------------------------------------------------

    set_expression: $ => choice(
      $.set_union_expression,
      $.set_difference_expression,
      $._set_inter,
    ),

    set_union_expression: $ => prec.left(seq(
      field('left', $.set_expression),
      '|',
      field('right', $.set_expression),
    )),

    set_difference_expression: $ => prec.left(seq(
      field('left', $.set_expression),
      '-',
      field('right', $.set_expression),
    )),

    _set_inter: $ => choice($.set_intersection_expression, $._set_operand),

    set_intersection_expression: $ => prec.left(seq(
      field('left', $._set_inter),
      '&',
      field('right', $._set_inter),
    )),

    _set_operand: $ => choice(
      $.value,
      $.set_literal,
      $.parenthesized_set,
    ),

    parenthesized_set: $ => seq('(', $.set_expression, ')'),

    set_literal: $ => seq(
      '{',
      optional(seq(sep1($.set_item, ','), optional(','))),
      '}',
    ),

    set_item: $ => choice($.value, $.spread),

    spread: $ => seq('...', $.set_expression),

    value: $ => choice($.path, $.string, $.integer, $.boolean),

    path: $ => seq($.ident, repeat(seq('.', $.ident))),


    // ------------------------------------------------------------------
    // Lexical terminals
    // ------------------------------------------------------------------

    ident: $ => /[A-Za-z_][A-Za-z0-9_]*/,

    integer: $ => choice('0', /[1-9][0-9]*/),

    boolean: $ => choice('true', 'false'),

    // The closing quote is optional so a half-typed string still lexes
    // and recovers at the line end (upstream refuses it; superset
    // acceptance recorded in COVERAGE.md). Interiors use immediate
    // tokens: no whitespace, comments or keyword lexing inside a string.
    string: $ => seq(
      '"',
      repeat(choice(
        $.escape_sequence,
        token.immediate(/[^"\\\n\r]+/),
      )),
      optional('"'),
    ),

    escape_sequence: $ => token.immediate(/\\[^\n\r]/),

    comment: $ => /\/\/[^\n\r]*/,
  },
});

function sep1(rule, separator) {
  return seq(rule, repeat(seq(separator, rule)));
}
