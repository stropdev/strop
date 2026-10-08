// tree-sitter-sovel — presentation grammar for Sovel 0.1-draft.4.
//
// Provenance: hand-authored against the Sovel handover snapshot
//   repository: /home/tarek/workspace/sovel/sovel-handover
//   revision:   eab2ce3290bb32fb32872d32890c313ee1828df2 ("Authorize
//               editor-only Sovel syntax preview and record Strop handoff")
//   syntax snapshot: 0.1-draft.4 (spec/grammar.ebnf + spec/LEXING-AND-PARSING.md)
//   generator: tree-sitter-cli 0.25.10, `tree-sitter generate --abi 15`
//   runtime: tree-sitter 0.25.10 (ABI 13–15)
//
// This is a *presentation* grammar for Strop's experimental syntax
// highlighting (Strop plan 0070). A colored or error-free tree is NOT proof
// of source validity, safety, macro hygiene or compilability; the
// authoritative frontend is gated behind Sovel's G-LANGUAGE milestone.
//
// Documented disambiguation policies (LEXING-AND-PARSING.md §"Decisions"):
//  - Expression generics always use `::<>`; `<` in expression position is a
//    comparison, `>>` never lexes (nested generic closings are two `>`).
//  - `2.seconds()` / `2.0` / `1..3` fall out of the float token requiring a
//    digit after the point; tuple projection `.0` reuses the integer token.
//  - `||` starts a lambda only where an expression begins; otherwise it is
//    Boolean OR. Or-patterns in lambda parameters need parentheses.
//  - Struct literals in control heads lose to the body block via negative
//    dynamic precedence on `struct_expression` (spec excludes them there;
//    the parenthesized form still parses, just at lower precedence).
//  - Nullary constructor patterns must be qualified (`Option::None`); bare
//    identifiers in pattern position are bindings, so
//    `constructor_pattern` without a payload requires a qualified path.
//  - No external scanner: block comments are non-nesting and strings are
//    single-line by specification, so regex tokens suffice.

const PREC = {
  lambda: 1,
  or: 2,
  and: 3,
  equality: 4,
  comparison: 5,
  range: 6,
  additive: 7,
  multiplicative: 8,
  unary: 9,
  postfix: 10,
  call: 11,
};

// Binary operators mapped to precedence levels (spec order: ||, &&, ==/!=,
// </<=/>/>=, ../..=, +/-, *//%). Equality/comparison/range do not chain in
// the authoritative semantics; a presentation grammar associates left so
// editing tolerance stays high.
const BINARY = [
  ['||', PREC.or],
  ['&&', PREC.and],
  ['==', PREC.equality],
  ['!=', PREC.equality],
  ['<', PREC.comparison],
  ['<=', PREC.comparison],
  ['>', PREC.comparison],
  ['>=', PREC.comparison],
  ['..', PREC.range],
  ['..=', PREC.range],
  ['+', PREC.additive],
  ['-', PREC.additive],
  ['*', PREC.multiplicative],
  ['/', PREC.multiplicative],
  ['%', PREC.multiplicative],
];

/// `punctuation, item, punctuation, ..., [punctuation]` with the separator
/// rule named so corpus trees stay readable.
function sep(rule, separator) {
  return seq(rule, repeat(seq(separator, rule)), optional(separator));
}

function sep1(rule, separator) {
  return seq(rule, repeat(seq(separator, rule)));
}

/// Possibly-empty separated list (EBNF `[ x, { ",", x } ]`-style slots).
function list0($, rule, separator) {
  return optional(sep(rule, separator));
}


module.exports = grammar({
  name: 'sovel',

  extras: $ => [/\s/, $.comment],

  word: $ => $.identifier,

  conflicts: $ => [
    // `Type { ... }` in a control head: body block wins (dynamic
    // precedence below), but both readings must be explored.
    [$._primary_expression, $.struct_expression],
    // `{ ... }` closing a statement list: nested block as the last
    // statement versus as the block's result expression.
    [$._block_statement, $._primary_expression],
    // Bare identifier in pattern position: binding or the start of a
    // qualified constructor path (`Option::None`, `Point { x }`).
    [$.binding_pattern, $.type_path],
    [$.type_path, $.constructor_pattern],
    // `|| -> T ! E { … }`: an effect row on the lambda result versus
    // unary `!` opening the body expression.
    [$.effect_atom, $.value_path],
    [$.effect_atom, $.module_path],
    [$.module_path, $.value_path],
    [$.module_path, $.type_path, $.value_path],
    [$.type_path, $.value_path],
    [$.type_atom, $._primary_expression],
    // Type versus expression spellings explored while GLR decides whether
    // a `{` after `!` is an effect set or a body block; exactly one
    // reading survives on valid input.
    [$.unit_type, $.unit_expression],
  ],

  rules: Object.assign(
    require('./rules/declarations')({ sep, sep1, list0 }),
    require('./rules/expressions')({ PREC, BINARY, sep, sep1, list0 }),
  ),
});
