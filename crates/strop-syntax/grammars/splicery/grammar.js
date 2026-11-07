// tree-sitter-splicery — presentation grammar for Splicery /1.
//
// Provenance: hand-authored against the Splicery checkout
//   repository: /home/tarek/workspace/splicery (read-only; MIT,
//               "Writfold authors")
//   revision:   c696b6e ("Correct final SG00 receipt paths"), dirty
//               working tree under active SG-gate work — implemented
//               against the committed spec only
//   sources:    spec/splicery.ebnf (normative, 24 productions),
//               spec/language.md §Lexer,
//               crates/splicery-syntax/src/{lexer,parser} as the
//               token-boundary reference; the frozen-replay shape comes
//               from tools/check_language.py
//   generator:  tree-sitter-cli 0.25.10, `tree-sitter generate --abi 15`
//   runtime:    tree-sitter 0.25.10 (ABI 13–15)
//
// Corpus snippets under test/corpus/ are original Strop-side test inputs
// written against the spec families (0070 convention; no Splicery
// fixture text is vendored).
//
// This is a *presentation* grammar for Strop's syntax highlighting
// (Strop plan 0083). A colored or error-free tree is NOT proof of a
// valid recipe, exact-match inventory, or any Splicery gate.
//
// Documented disambiguation policies (spec/language.md §Lexer):
//  - Contextual spellings are NOT global keywords: `text`, `line`,
//    `like`, `within`, `expect`, `and`, `to` exist only inside fixed
//    productions. An identifier spelled `text` elsewhere stays a name.
//  - `:=` is one token; HANDLE is `@` + ASCII name (two grammar tokens
//    so the marker and the name can take different classes).
//  - `rx"…"` is ONE raw single-line token (a quote ends it; `#` and
//    `\` are body bytes). `hex"…"` accepts any run of hex digits — odd
//    counts are a semantic refusal upstream, not a parse strategy.
//  - NAT keeps the spec shape `0 | [1-9][0-9]*` (leading zeros are an
//    upstream refusal; they lex as two nats and recover visibly).
//  - Strings may be unterminated to the line end (upstream refuses;
//    superset recorded in COVERAGE.md) so half-typed recipes stay
//    editable.
//  - HEREDOC is the one external scanner (src/scanner.c): the opener
//    `<<'TAG'` + LF/CRLF is scanned externally so the scanner can
//    record the tag, then the body and terminator line are emitted as
//    `heredoc_body` / `heredoc_end` tokens. Body ends at the first
//    physical line equal to TAG (terminator excluded, may end at EOF,
//    empty body valid, no stripping/interpolation/escapes).

module.exports = grammar({
  name: 'splicery',

  word: $ => $.name,

  externals: $ => [
    $.heredoc_start,
    $.heredoc_body,
    $.heredoc_end,
  ],

  extras: $ => [/[\t\n\r ]/, $.comment],

  rules: {
    document: $ => choice(
      $.edit_file,
      $.create_file,
      $.rename_file,
      $.remove_file,
      $.fragment,
    ),

    edit_file: $ => seq(
      'edit',
      field('path', $.string),
      optional($.raw_profile),
      '{',
      repeat($.statement),
      '}',
    ),

    create_file: $ => seq(
      'create',
      field('path', $.string),
      optional($.raw_profile),
      'with',
      field('payload', $.payload),
    ),

    rename_file: $ => seq(
      'rename',
      field('from', $.string),
      'to',
      field('to', $.string),
    ),

    remove_file: $ => seq('remove', field('path', $.string)),

    fragment: $ => repeat1($.statement),

    raw_profile: $ => seq('as', 'bytes'),

    statement: $ => choice(
      $.binding_statement,
      $.replace_statement,
      $.before_statement,
      $.after_statement,
      $.delete_statement,
      $.iteration,
    ),

    binding_statement: $ => seq(
      field('handle', $.handle),
      ':=',
      field('selector', $.selector),
    ),

    replace_statement: $ => seq(
      'replace',
      field('target', $.selector),
      'with',
      field('payload', $.payload),
    ),

    before_statement: $ => seq(
      'before',
      field('target', $.selector),
      'insert',
      field('payload', $.payload),
    ),

    after_statement: $ => seq(
      'after',
      field('target', $.selector),
      'insert',
      field('payload', $.payload),
    ),

    delete_statement: $ => seq('delete', field('target', $.selector)),

    iteration: $ => seq(
      'each',
      field('handle', $.handle),
      'in',
      field('collection', $.collection),
      '{',
      repeat(choice(
        $.replace_statement,
        $.before_statement,
        $.after_statement,
        $.delete_statement,
      )),
      '}',
    ),

    collection: $ => choice($.line_matches, $.frozen_collection),

    line_matches: $ => seq(
      'line_matches',
      field('pattern', $.regex),
      optional(seq('within', field('window', $.reference))),
      'expect',
      field('count', $.nat),
    ),

    frozen_collection: $ => seq('frozen', field('source', $.reference)),

    selector: $ => choice(
      $.reference,
      $.one_selector,
      $.between_selector,
      $.range_selector,
    ),

    reference: $ => seq(
      field('handle', $.handle),
      optional(seq('.', field('group', $.name))),
    ),

    one_selector: $ => seq(
      'one',
      choice(
        seq('text', field('text', $.string)),
        seq('line', field('line', $.string)),
        field('hex', $.hex_literal),
      ),
      optional(seq('within', field('window', $.reference))),
    ),

    between_selector: $ => seq(
      'between',
      field('start', $.reference),
      'and',
      field('end', $.reference),
    ),

    range_selector: $ => seq(
      'range',
      '[',
      field('start', $.nat),
      ',',
      field('end', $.nat),
      ')',
    ),

    payload: $ => choice($.piece, $.payload_list),

    payload_list: $ => seq(
      '[',
      optional(seq(sep1($.piece, ','), optional(','))),
      ']',
    ),

    piece: $ => choice(
      $.string,
      $.heredoc,
      $.hex_literal,
      $.keep_piece,
      $.lines_like_piece,
      $.indent_piece,
    ),

    keep_piece: $ => seq('keep', '(', field('source', $.reference), ')'),

    lines_like_piece: $ => seq(
      'lines',
      'like',
      field('source', $.reference),
      '[',
      optional(sep1(field('line', $.string), ',')),
      ']',
    ),

    indent_piece: $ => seq(
      'indent',
      '(',
      field('prefix', $.string),
      ',',
      field('payload', $.payload),
      ')',
    ),

    // `@` and the name are separate tokens so the marker and the name
    // can take different highlight classes.
    handle: $ => seq('@', field('name', $.name)),

    // ------------------------------------------------------------------
    // Lexical terminals
    // ------------------------------------------------------------------

    name: $ => /[A-Za-z_][A-Za-z0-9_]*/,

    nat: $ => choice('0', /[1-9][0-9]*/),

    // One raw token: `#`, `\` and `"` handling inside the body belong
    // to the regex engine, not this lexer. A `"` ends the token; the
    // body never crosses a line break.
    regex: $ => /rx"[^"\n\r]*"/,

    // Odd digit counts are a semantic refusal upstream; the grammar
    // accepts any run of hex digits.
    hex_literal: $ => /hex"[0-9a-fA-F]*"/,

    // The closing quote is optional so a half-typed string still lexes
    // and recovers at the line end. Interiors use immediate tokens: no
    // whitespace, comments or keyword lexing inside a string.
    string: $ => seq(
      '"',
      repeat(choice(
        $.escape_sequence,
        token.immediate(/[^"\\\n\r]+/),
      )),
      optional('"'),
    ),

    escape_sequence: $ => token.immediate(/\\[^\n\r]/),

    heredoc: $ => seq(
      $.heredoc_start,
      optional($.heredoc_body),
      $.heredoc_end,
    ),

    comment: $ => /#[^\n\r]*/,
  },
});

function sep1(rule, separator) {
  return seq(rule, repeat(seq(separator, rule)));
}
