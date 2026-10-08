// Declaration-side rules for the Sovel presentation grammar: items, type
// declarations, traits/impls, extern blocks, signatures, generics, types,
// origins and effect rows. Loaded by ../grammar.js — see its header for
// provenance and the disambiguation policy notes.
//
// `helpers` provides the shared list combinators from grammar.js.
module.exports = (helpers) => {
  const { sep, sep1, list0 } = helpers;
  return {
    source_file: $ => repeat($._item),

    // ------------------------------------------------------------------
    // Items (EBNF: item, module_decl, import_decl, attribute)
    // ------------------------------------------------------------------

    // EBNF `item = { attribute }, [ visibility ], item_body`: the shared
    // prefix lives here, not inside each declaration rule.
    _item: $ => seq(repeat($.attribute), optional('pub'), $._item_body),

    _item_body: $ => choice(
      $.module_declaration,
      $.import_declaration,
      $.struct_declaration,
      $.enum_declaration,
      $.newtype_declaration,
      $.type_alias,
      $.trait_declaration,
      $.impl_declaration,
      $.function_declaration,
      $.const_declaration,
      $.effect_declaration,
      $.macro_declaration,
      $.item_macro_call,
      $.item_splice,
      $.extern_block,
    ),

    module_declaration: $ => seq(
      'module',
      $.module_path,
      choice(';', seq('{', repeat($._item), '}')),
    ),

    import_declaration: $ => seq(
      'import',
      $.module_path,
      optional(seq('as', $.identifier)),
      ';',
    ),

    module_path: $ => sep1($.identifier, '::'),

    attribute: $ => seq(
      '#',
      '[',
      $.module_path,
      optional(choice(
        seq('(', list0($, $.argument, ','), ')'),
        seq('=', $._expression),
      )),
      ']',
    ),

    // ------------------------------------------------------------------
    // Type declarations
    // ------------------------------------------------------------------

    struct_declaration: $ => seq(
      'struct',
      field('name', $.identifier),
      optional($.generic_parameters),
      optional($.where_clause),
      '{',
      optional($.field_declarations),
      '}',
    ),

    field_declarations: $ => sep($.field_declaration, ','),

    field_declaration: $ => seq(
      repeat($.attribute),
      optional('pub'),
      field('name', $.identifier),
      ':',
      $._type,
    ),

    enum_declaration: $ => seq(
      'enum',
      field('name', $.identifier),
      optional($.generic_parameters),
      optional($.where_clause),
      '{',
      sep($.variant_declaration, ','),
      '}',
    ),

    variant_declaration: $ => seq(
      repeat($.attribute),
      field('name', $.identifier),
      optional(choice(
        seq('(', $.type_list, ')'),
        seq('{', optional($.field_declarations), '}'),
      )),
    ),

    newtype_declaration: $ => seq(
      'newtype',
      field('name', $.identifier),
      optional($.generic_parameters),
      '=',
      $._type,
      optional($.where_clause),
      ';',
    ),

    type_alias: $ => seq(
      'type',
      field('name', $.identifier),
      optional($.generic_parameters),
      '=',
      $._type,
      ';',
    ),

    const_declaration: $ => seq(
      'const',
      field('name', $.identifier),
      ':',
      $._type,
      '=',
      $._expression,
      ';',
    ),

    effect_declaration: $ => seq(
      'effect',
      field('name', $.identifier),
      optional(seq('(', sep($.origin_parameter, ','), ')')),
      ';',
    ),

    origin_parameter: $ => seq('origin', $.identifier),

    // ------------------------------------------------------------------
    // Traits and impls
    // ------------------------------------------------------------------

    trait_declaration: $ => seq(
      optional('dyn'),
      'trait',
      field('name', $.identifier),
      optional($.generic_parameters),
      optional(seq(':', $.trait_bounds)),
      optional($.where_clause),
      '{',
      repeat($._trait_member),
      '}',
    ),

    _trait_member: $ => seq(
      repeat($.attribute),
      choice(
        seq($.function_signature, choice(';', $.block)),
        seq('type', field('name', $.identifier), optional(seq(':', $.trait_bounds)), ';'),
        seq('const', field('name', $.identifier), ':', $._type, optional(seq('=', $._expression)), ';'),
      ),
    ),

    impl_declaration: $ => seq(
      'impl',
      optional($.generic_parameters),
      $._type,
      optional(seq('for', $._type)),
      optional($.where_clause),
      '{',
      repeat($._impl_member),
      '}',
    ),

    _impl_member: $ => seq(
      repeat($.attribute),
      optional('pub'),
      choice(
        $.function_declaration,
        seq('type', field('name', $.identifier), '=', $._type, ';'),
        $.const_declaration,
        $.delegation_declaration,
      ),
    ),

    delegation_declaration: $ => seq('delegate', $.trait_type, 'to', $.place, ';'),

    trait_bounds: $ => sep1($.trait_type, '+'),

    trait_type: $ => seq($.type_path, optional($.generic_arguments)),

    // ------------------------------------------------------------------
    // Foreign declarations (EBNF: extern_block, extern_member)
    // ------------------------------------------------------------------

    extern_block: $ => seq(
      'extern',
      field('abi', $.string_literal),
      field('name', $.identifier),
      '{',
      repeat($._extern_member),
      '}',
    ),

    _extern_member: $ => seq(
      repeat($.attribute),
      optional('pub'),
      choice(
        seq('type', field('name', $.identifier), ';'),
        $.extern_function,
      ),
    ),

    extern_function: $ => seq(
      'fn',
      field('name', $.identifier),
      '(',
      list0($, $.extern_parameter, ','),
      ')',
      '->',
      $._type,
      ';',
    ),

    extern_parameter: $ => seq(field('name', $.identifier), ':', $._type),

    // ------------------------------------------------------------------
    // Functions, macros, signatures (EBNF: function_signature, macro_decl)
    // ------------------------------------------------------------------

    function_declaration: $ => seq($.function_signature, $.block),

    function_signature: $ => seq(
      optional('comptime'),
      'fn',
      field('name', $.identifier),
      optional($.generic_parameters),
      $.parameter_list,
      optional($.context_parameters),
      '->',
      field('return_type', $._type),
      optional($.effect_clause),
      optional($.where_clause),
    ),

    parameter_list: $ => seq('(', list0($, $.parameter, ','), ')'),

    parameter: $ => choice(
      $.receiver,
      seq($._pattern, ':', $._type),
    ),

    receiver: $ => seq(optional(seq('&', optional('mut'))), 'self'),

    context_parameters: $ => seq('using', sep1($.context_parameter, ',')),

    context_parameter: $ => seq(field('name', $.identifier), ':', $._type),

    macro_declaration: $ => seq(
      'macro',
      field('name', $.identifier),
      optional($.generic_parameters),
      $.parameter_list,
      optional($.context_parameters),
      '->',
      field('return_type', $._type),
      optional($.effect_clause),
      optional($.where_clause),
      $.block,
    ),

    // ------------------------------------------------------------------
    // Generics, where clauses (EBNF: generic_parameters, where_clause)
    // ------------------------------------------------------------------

    generic_parameters: $ => seq('<', sep($.generic_parameter, ','), '>'),

    generic_parameter: $ => choice(
      seq($.type_parameter, optional(seq(':', $.trait_bounds))),
      $.effect_parameter,
      $.generic_origin_parameter,
      $.const_parameter,
    ),

    type_parameter: $ => $.identifier,

    effect_parameter: $ => seq('effects', $.identifier),

    generic_origin_parameter: $ => seq(choice('origin', 'origins'), $.identifier),

    const_parameter: $ => seq('const', $.identifier, ':', $._type),

    generic_arguments: $ => seq('<', sep($.generic_argument, ','), '>'),

    generic_argument: $ => choice(
      $._type,
      seq('effects', $.effect_expression),
      seq('origin', $.origin_path),
      seq('origins', $.origin_bound),
      seq('const', $.constant_argument),
      seq($.identifier, '=', $._type),
    ),

    constant_argument: $ => choice(
      $.integer_literal,
      $.boolean_literal,
      $.identifier,
    ),

    where_clause: $ => seq('where', sep($.constraint, ',')),

    constraint: $ => choice(
      seq($._type, ':', $.trait_bounds),
      seq('effects', $.identifier, 'excludes', $.effect_label),
      seq('origin', $.origin_path, 'outlives', $.origin_path),
      seq('origin', $.origin_path, 'disjoint', $.origin_path),
      seq($._type, '=', $._type),
    ),

    // ------------------------------------------------------------------
    // Types (EBNF: type, type_prefix, type_atom, abstract_type)
    // ------------------------------------------------------------------

    // `from` binds to the annotated value (spec decision 2): without
    // parentheses the origin clause attaches to the innermost type.
    _type: $ => prec.right(seq($._type_prefix, optional($.origin_clause))),

    _type_prefix: $ => choice(
      $.reference_type,
      $.function_type,
      $.abstract_type,
      $.type_atom,
    ),

    reference_type: $ => seq(
      '&',
      optional('mut'),
      choice($.type_atom, $.abstract_type),
    ),

    function_type: $ => prec.right(seq(
      optional(seq('for', $.generic_parameters)),
      'fn',
      optional($.call_mode),
      '(',
      list0($, $.function_type_parameter, ','),
      ')',
      optional($.context_parameters),
      '->',
      $._type,
      optional($.effect_clause),
    )),

    call_mode: $ => choice('mut', 'once'),

    function_type_parameter: $ => seq(
      optional(seq(field('name', $.identifier), ':')),
      $._type,
    ),

    abstract_type: $ => seq(choice('impl', 'dyn'), $._abstract_bound),

    _abstract_bound: $ => choice(
      $.trait_bounds,
      seq('(', $.function_type, ')'),
      $.function_type,
    ),

    type_atom: $ => choice(
      seq($.type_path, optional($.generic_arguments)),
      $.unit_type,
      $.parenthesized_type,
      $.tuple_type,
      $.array_type,
      $.syntax_splice,
    ),

    unit_type: $ => seq('(', ')'),

    parenthesized_type: $ => seq('(', $._type, ')'),

    tuple_type: $ => seq('(', $._type, ',', optional($.type_list), ')'),

    array_type: $ => seq('[', $._type, optional(seq(';', $.constant_argument)), ']'),

    type_list: $ => sep($._type, ','),

    type_path: $ => seq(
      choice($.identifier, 'Self'),
      repeat(seq(choice('::', '.'), $.identifier)),
    ),

    // ------------------------------------------------------------------
    // Origins and effects (EBNF: origin_clause, effect_clause, rows)
    // ------------------------------------------------------------------

    origin_clause: $ => seq('from', $.origin_bound),

    origin_bound: $ => choice(
      $.origin_path,
      seq('{', list0($, $.origin_member, ','), '}'),
    ),

    origin_member: $ => choice(
      $.origin_path,
      $.origin_tail,
    ),

    origin_tail: $ => seq('..', $.identifier),

    origin_path: $ => seq(
      choice($.identifier, 'self', 'static'),
      repeat(seq('.', $.identifier)),
    ),

    effect_clause: $ => seq('!', $.effect_expression),

    effect_expression: $ => sep1($.effect_atom, '+'),

    effect_atom: $ => choice(
      $.identifier,
      $.effect_set,
      seq('(', $.effect_expression, ')'),
    ),

    // After `!`, `{…}` is an effect set, not a body block (e.g.
    // `|| -> T ! {} [0]`): the set reading carries higher precedence.
    effect_set: $ => prec(1, seq(
      '{',
      optional(choice(
        seq(sep1($.effect_label, ','), optional(seq(',', $.row_tail)), optional(',')),
        seq($.row_tail, optional(',')),
      )),
      '}',
    )),

    row_tail: $ => seq('..', $.identifier),

    effect_label: $ => choice(
      seq('fail', '(', $._type, ')'),
      seq('drop', '(', $._type, ')'),
      seq($.effect_name, optional(seq('(', sep1($.origin_path, ','), ')'))),
    ),

    effect_name: $ => choice($.module_path, 'suspend', 'cancel', 'spawn'),
  };
};
