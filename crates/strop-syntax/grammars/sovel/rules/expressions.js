// Expression-side rules for the Sovel presentation grammar: statements,
// the expression ladder, control-flow blocks, patterns, macro token
// trees, quotes/splices and the lexical terminals. Loaded by
// ../grammar.js — see its header for provenance and policy notes.
module.exports = (helpers) => {
  const { PREC, BINARY, sep, sep1, list0 } = helpers;
  return {
    // ------------------------------------------------------------------
    // Statements and blocks (EBNF: block, statement, place)
    // ------------------------------------------------------------------

    block: $ => seq('{', repeat($._statement), optional($._expression), '}'),

    _statement: $ => choice(
      $.let_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.assignment_statement,
      $.expression_statement,
      $._block_statement,
    ),

    let_statement: $ => seq(
      'let',
      optional('mut'),
      $._pattern,
      optional(seq(':', $._type)),
      '=',
      $._expression,
      ';',
    ),

    return_statement: $ => seq('return', optional($._expression), ';'),

    break_statement: $ => seq('break', ';'),

    continue_statement: $ => seq('continue', ';'),

    // The EBNF restricts the target to `place`; a presentation grammar
    // takes any expression on the left (tolerant, and it keeps statement
    // recovery local — the authoritative checker owns target validity).
    assignment_statement: $ => seq($._expression, $.assignment_operator, $._expression, ';'),

    assignment_operator: $ => choice('=', '+=', '-=', '*=', '/=', '%='),

    expression_statement: $ => seq($._expression, ';'),

    _block_statement: $ => choice(
      $.block,
      $.if_expression,
      $.match_expression,
      $.while_expression,
      $.for_expression,
      $.loop_expression,
      $.try_expression,
      $.region_expression,
      $.tasks_expression,
      $.unsafe_expression,
    ),

    place: $ => seq(
      choice($.identifier, 'self', seq('*', $.unary_expression)),
      repeat(choice(
        seq('.', $.identifier),
        seq('[', $._expression, ']'),
      )),
    ),

    // ------------------------------------------------------------------
    // Expressions (EBNF: expression ladder, primary_expression)
    // ------------------------------------------------------------------

    _expression: $ => choice(
      $.lambda_expression,
      $.fail_expression,
      $.unary_expression,
      $.spawn_expression,
      $.binary_expression,
      $.call_expression,
      $.member_expression,
      $.index_expression,
      $.propagation_expression,
      $.macro_call,
      $._primary_expression,
    ),

    // `move || …` is a moved lambda, not unary `move` on a lambda: the
    // explicit-`move` lambda variant carries higher precedence than both
    // the plain lambda and unary `move`, so the modifier reading wins.
    lambda_expression: $ => choice(
      prec(PREC.lambda + 1, seq('move', $._lambda_body)),
      prec(PREC.lambda, $._lambda_body),
    ),

    _lambda_body: $ => seq(
      choice(
        '||',
        seq('|', sep($.lambda_parameter, ','), '|'),
      ),
      optional(seq('->', $._type, optional($.effect_clause))),
      $._expression,
    ),

    lambda_parameter: $ => seq($._atomic_pattern, optional(seq(':', $._type))),

    fail_expression: $ => prec(PREC.lambda, seq('fail', $._expression)),

    // `move || …` is a moved lambda, not unary `move` on a lambda: the
    // unary `move` reading carries strictly lower precedence than
    // `lambda_expression` so the modifier reading wins the conflict.
    unary_expression: $ => choice(
      prec(PREC.unary, seq(choice('!', '-', '*', 'await'), $._expression)),
      prec(PREC.unary, seq('&', optional('mut'), $._expression)),
      prec(PREC.lambda - 1, seq('move', $._expression)),
    ),

    spawn_expression: $ => prec(PREC.unary, seq('spawn', $._expression)),

    binary_expression: $ => choice(...BINARY.map(([operator, precedence]) =>
      prec.left(precedence, seq($._expression, operator, $._expression)),
    )),

    call_expression: $ => prec(PREC.call, seq(
      field('function', $._expression),
      $.argument_list,
      optional($.context_arguments),
    )),

    argument_list: $ => seq('(', list0($, $.argument, ','), ')'),

    argument: $ => seq(optional(seq($.identifier, '=')), $._expression),

    context_arguments: $ => seq('using', '(', sep($.context_argument, ','), ')'),

    context_argument: $ => seq($.identifier, '=', $._expression),

    member_expression: $ => prec(PREC.call, seq(
      field('value', $._expression),
      '.',
      field('member', choice(
        seq($.identifier, optional(seq('::', $.generic_arguments))),
        $.integer_literal,
      )),
    )),

    index_expression: $ => prec(PREC.call, seq(
      field('value', $._expression),
      '[',
      $._expression,
      ']',
    )),

    propagation_expression: $ => prec(PREC.call, seq($._expression, '?')),

    _primary_expression: $ => choice(
      $.literal,
      $.value_path,
      $.unit_expression,
      $.parenthesized_expression,
      $.tuple_expression,
      $.array_expression,
      $.struct_expression,
      $.block,
      $.if_expression,
      $.match_expression,
      $.while_expression,
      $.for_expression,
      $.loop_expression,
      $.try_expression,
      $.region_expression,
      $.tasks_expression,
      $.unsafe_expression,
      $.quote_expression,
      $.syntax_splice,
      $.comptime_expression,
    ),

    value_path: $ => prec.right(seq(
      choice($.identifier, 'self', 'Self', 'drop'),
      repeat(seq('::', $.identifier)),
      optional(seq('::', $.generic_arguments)),
    )),

    unit_expression: $ => seq('(', ')'),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    tuple_expression: $ => seq('(', $._expression, ',', optional($.expression_list), ')'),

    expression_list: $ => sep($._expression, ','),

    array_expression: $ => seq(
      '[',
      optional(choice(
        $.expression_list,
        seq($._expression, ';', $._expression),
      )),
      ']',
    ),

    // Control-head braces beat a struct literal: the body block is the
    // preferred reading (spec decision 6), so the struct reading carries
    // negative dynamic precedence. Parenthesized heads remain the only
    // unambiguous way to test a struct value.
    struct_expression: $ => prec.dynamic(-1, seq(
      $.value_path,
      '{',
      list0($, $.field_initializer, ','),
      '}',
    )),

    field_initializer: $ => choice(
      seq($.identifier, optional(seq(':', $._expression))),
      $.syntax_splice,
    ),

    if_expression: $ => prec.right(seq(
      'if',
      $._condition,
      $.block,
      optional(seq('else', choice($.block, $.if_expression))),
    )),

    _condition: $ => choice(
      $._expression,
      seq('let', $._pattern, '=', $._expression),
    ),

    match_expression: $ => seq(
      'match',
      $._expression,
      '{',
      sep($.match_arm, ','),
      '}',
    ),

    match_arm: $ => seq(
      $._pattern,
      optional(seq('if', $._expression)),
      '=>',
      $._expression,
    ),

    while_expression: $ => seq('while', $._condition, $.block),

    for_expression: $ => seq('for', $._pattern, 'in', $._expression, $.block),

    loop_expression: $ => seq('loop', $.block),

    try_expression: $ => seq('try', $.block, repeat1($.catch_clause)),

    catch_clause: $ => seq(
      'catch',
      $._pattern,
      optional(seq('if', $._expression)),
      $.block,
    ),

    region_expression: $ => seq(
      'region',
      field('name', $.identifier),
      ':',
      $._type,
      'in',
      $._expression,
      $.block,
    ),

    tasks_expression: $ => seq(
      'tasks',
      optional(seq('on', $._expression)),
      optional(seq('within', $._expression)),
      $.block,
    ),

    unsafe_expression: $ => seq(
      'unsafe',
      '(',
      field('contract', $.module_path),
      ')',
      $.block,
    ),

    comptime_expression: $ => seq('comptime', $.block),

    // ------------------------------------------------------------------
    // Macro calls, token trees, quotes and splices (EBNF §macro/quote)
    // ------------------------------------------------------------------

    macro_call: $ => seq($.module_path, '!', $.token_group),

    item_macro_call: $ => seq($.macro_call, ';'),

    quote_expression: $ => choice(
      seq('quote', $.block),
      seq('quote', 'expr', '(', $._expression, ')'),
      seq('quote', 'block', $.block),
      seq('quote', 'type', '(', $._type, ')'),
      seq('quote', 'pattern', '(', $._pattern, ')'),
      seq('quote', 'items', '{', repeat($._item), '}'),
    ),

    syntax_splice: $ => choice(
      seq('$', $.identifier),
      seq('$', '{', $._expression, '}'),
    ),

    item_splice: $ => seq($.syntax_splice, ';'),

    token_group: $ => choice(
      seq('(', repeat($.token_tree), ')'),
      seq('[', repeat($.token_tree), ']'),
      seq('{', repeat($.token_tree), '}'),
    ),

    // Balanced macro input can be an arbitrary DSL (spec decision 10):
    // delimiters nest as groups; everything else is one opaque token run.
    // Strings and characters stay intact so delimiters inside them cannot
    // unbalance a group. Comments are `extras` and never become tokens.
    token_tree: $ => choice(
      $.token_group,
      $.string_literal,
      $.char_literal,
      $._macro_token,
    ),

    _macro_token: $ => token(prec(-1, /[^\s()\[\]{}"']+/)),

    // ------------------------------------------------------------------
    // Patterns (EBNF: pattern, atomic_pattern, constructor_pattern)
    // ------------------------------------------------------------------

    _pattern: $ => choice(
      $.or_pattern,
      $._atomic_pattern,
    ),

    or_pattern: $ => seq($._atomic_pattern, repeat1(seq('|', $._atomic_pattern))),

    _atomic_pattern: $ => choice(
      $.wildcard_pattern,
      $.literal,
      $.binding_pattern,
      $.tuple_pattern,
      $.parenthesized_pattern,
      $.constructor_pattern,
      $.slice_pattern,
      $.syntax_splice,
    ),

    wildcard_pattern: $ => '_',

    binding_pattern: $ => seq(
      optional(seq('ref', optional('mut'))),
      $.identifier,
      optional(seq('@', $._atomic_pattern)),
    ),

    tuple_pattern: $ => choice(
      seq('(', ')'),
      seq('(', $._pattern, ',', optional($.pattern_list), ')'),
    ),

    parenthesized_pattern: $ => seq('(', $._pattern, ')'),

    pattern_list: $ => sep($._pattern, ','),

    // Bare identifiers bind (spec decision 7); a payload-free constructor
    // pattern therefore requires a qualified path.
    constructor_pattern: $ => choice(
      seq($.type_path, choice(
        seq('(', optional($.pattern_list), ')'),
        seq('{', optional($.field_patterns), '}'),
      )),
      seq($.identifier, repeat1(seq(choice('::', '.'), $.identifier))),
    ),

    field_patterns: $ => sep($.field_pattern, ','),

    field_pattern: $ => choice(
      seq($.identifier, optional(seq(':', $._pattern))),
      '..',
    ),

    slice_pattern: $ => seq('[', optional($.pattern_list), ']'),

    // ------------------------------------------------------------------
    // Lexical terminals (LEXING-AND-PARSING.md §"Encoding and lexical
    // terminals"): ASCII identifiers, no lifetimes, no raw strings, no
    // interpolation, non-nesting block comments.
    // ------------------------------------------------------------------

    literal: $ => choice(
      $.integer_literal,
      $.float_literal,
      $.string_literal,
      $.char_literal,
      $.boolean_literal,
    ),

    // `_` alone is the wildcard pattern, never an identifier.
    identifier: $ => /[A-Za-z][A-Za-z0-9_]*|_[A-Za-z0-9_]+/,

    integer_literal: $ => token(choice(
      /0x[0-9a-fA-F](_?[0-9a-fA-F])*/,
      /0b[01](_?[01])*/,
      /0o[0-7](_?[0-7])*/,
      /[0-9](_?[0-9])*/,
    )),

    // A float needs a digit after the point, or an exponent. `1..3` and
    // `2.seconds()` therefore lex as integer + `..` / `.` + member.
    float_literal: $ => token(choice(
      /[0-9](_?[0-9])*\.[0-9](_?[0-9])*([eE][+-]?[0-9](_?[0-9])*)?/,
      /[0-9](_?[0-9])*[eE][+-]?[0-9](_?[0-9])*/,
    )),

    string_literal: $ => token(seq(
      '"',
      repeat(choice(
        /[^"\\\n]/,
        /\\[\\"nrt0]/,
        /\\u\{[0-9a-fA-F]+\}/,
      )),
      '"',
    )),

    char_literal: $ => token(seq(
      '\'',
      choice(
        /[^'\\\n]/,
        /\\[\\'nrt0]/,
        /\\u\{[0-9a-fA-F]+\}/,
      ),
      '\'',
    )),

    boolean_literal: $ => choice('true', 'false'),

    // `///` and `/** ... */` are the doc forms; block comments do NOT nest
    // in this snapshot. One `comment` token covers both line and block.
    comment: $ => token(choice(
      seq('//', /.*/),
      seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/'),
    )),
  };
};
