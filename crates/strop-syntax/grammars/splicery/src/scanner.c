// External scanner for Splicery heredocs — the grammar's only scanner
// (Strop plan 0083). A heredoc's terminator is a dynamic token (the
// tag spelled in the opener), which a regex token cannot express.
//
// Three tokens, in order:
//   heredoc_start  `<<'TAG'` + LF or CRLF — records TAG (≤ 64 ASCII
//                  identifier bytes) in the scanner state.
//   heredoc_body   raw content up to (excluding) the first physical
//                  line equal to TAG. May be absent (empty body). No
//                  stripping, interpolation, escapes or comment
//                  recognition — the body is one raw token.
//   heredoc_end    the terminator line: TAG + LF/CRLF, or TAG at EOF.
//
// Mirrors the reference lexer exactly (splicery-syntax
// lexer/literal.rs `heredoc`): a line with leading whitespace or a
// trailing suffix (`TAGX`) is content, not a terminator; `TAG\r` at
// EOF is not a terminator; the closing delimiter may end at EOF.

#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

// TAG is at most 64 identifier bytes (spec §Lexer).
#define MAX_TAG 64

enum TokenType {
  HEREDOC_START,
  HEREDOC_BODY,
  HEREDOC_END,
};

typedef struct {
  uint8_t tag_len;
  char tag[MAX_TAG];
} ScannerState;

void *tree_sitter_splicery_external_scanner_create(void) {
  ScannerState *state = calloc(1, sizeof(ScannerState));
  return state;
}

void tree_sitter_splicery_external_scanner_destroy(void *payload) {
  free(payload);
}

unsigned tree_sitter_splicery_external_scanner_serialize(
    void *payload, char *buffer) {
  ScannerState *state = payload;
  if (state->tag_len > MAX_TAG) {
    return 0;
  }
  buffer[0] = (char)state->tag_len;
  memcpy(buffer + 1, state->tag, state->tag_len);
  return (unsigned)state->tag_len + 1;
}

void tree_sitter_splicery_external_scanner_deserialize(
    void *payload, const char *buffer, unsigned length) {
  ScannerState *state = payload;
  state->tag_len = 0;
  if (length == 0) {
    return;
  }
  uint8_t tag_len = (uint8_t)buffer[0];
  if (tag_len > MAX_TAG || (unsigned)tag_len + 1 > length) {
    return;
  }
  state->tag_len = tag_len;
  memcpy(state->tag, buffer + 1, tag_len);
}

static bool is_tag_start(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_';
}

static bool is_tag_continue(int32_t c) {
  return is_tag_start(c) || (c >= '0' && c <= '9');
}
// Consume `<<'TAG'` + LF/CRLF. On success records the tag, marks the
// end after the line break and returns true. Any mismatch returns
// false, which rewinds the lexer to the token start. Leading
// whitespace is skipped as extras — the main lexer has not run yet at
// this position.
static bool scan_heredoc_start(TSLexer *lexer, ScannerState *state) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t' ||
         lexer->lookahead == '\n' || lexer->lookahead == '\r') {
    lexer->advance(lexer, true);
  }
  if (lexer->lookahead != '<') {
    return false;
  }
  lexer->advance(lexer, false);
  if (lexer->lookahead != '<') {
    return false;
  }
  lexer->advance(lexer, false);
  if (lexer->lookahead != '\'') {
    return false;
  }
  lexer->advance(lexer, false);
  if (!is_tag_start(lexer->lookahead)) {
    return false;
  }
  uint8_t len = 0;
  char tag[MAX_TAG];
  while (is_tag_continue(lexer->lookahead)) {
    if (len == MAX_TAG) {
      return false; // spec limit: tag exceeds 64 bytes
    }
    tag[len] = (char)lexer->lookahead;
    len++;
    lexer->advance(lexer, false);
  }
  if (lexer->lookahead != '\'') {
    return false;
  }
  lexer->advance(lexer, false);
  if (lexer->lookahead == '\r') {
    lexer->advance(lexer, false);
    if (lexer->lookahead != '\n') {
      return false; // lone CR is not a line break
    }
    lexer->advance(lexer, false);
  } else if (lexer->lookahead == '\n') {
    lexer->advance(lexer, false);
  } else {
    return false; // opener must be followed immediately by LF or CRLF
  }
  memcpy(state->tag, tag, len);
  state->tag_len = len;
  lexer->mark_end(lexer);
  lexer->result_symbol = HEREDOC_START;
  return true;
}

// The lexer is at a line start. Consume bytes while they match the
// tag; if every tag byte matches AND the next byte is LF, CRLF or EOF,
// leave the lexer there and return true (this line is the terminator).
static bool probe_terminator(TSLexer *lexer, const ScannerState *state) {
  for (uint8_t i = 0; i < state->tag_len; i++) {
    if (lexer->eof(lexer) || (char)lexer->lookahead != state->tag[i]) {
      return false;
    }
    lexer->advance(lexer, false);
  }
  if (lexer->eof(lexer)) {
    return true;
  }
  if (lexer->lookahead == '\n') {
    return true;
  }
  if (lexer->lookahead == '\r') {
    // Peek one further: only CRLF terminates; a lone CR is content.
    lexer->advance(lexer, false);
    return !lexer->eof(lexer) && lexer->lookahead == '\n';
  }
  return false;
}

// Consume the terminator line (probe already consumed the tag): the
// EOL, or nothing at EOF. Returns false if the EOL is malformed.
static bool finish_terminator(TSLexer *lexer) {
  if (lexer->eof(lexer)) {
    return true;
  }
  if (lexer->lookahead == '\n') {
    lexer->advance(lexer, false);
    return true;
  }
  if (lexer->lookahead == '\r') {
    lexer->advance(lexer, false);
    if (!lexer->eof(lexer) && lexer->lookahead == '\n') {
      lexer->advance(lexer, false);
      return true;
    }
    return false;
  }
  return false;
}

static bool consume_to_eol(TSLexer *lexer) {
  while (!lexer->eof(lexer)) {
    int32_t c = lexer->lookahead;
    lexer->advance(lexer, false);
    if (c == '\n') {
      return true;
    }
  }
  return false;
}

// The lexer is inside a heredoc (tag recorded). Walk physical lines,
// marking each line end as a candidate body end, until the terminator
// line: emit `heredoc_body` for everything before it, or
// `heredoc_end` for the terminator itself when the body is empty.
// With no terminator at all, the remaining input is the body so an
// unclosed heredoc still colors and recovers instead of cascading.
static bool scan_heredoc_body_or_end(TSLexer *lexer, ScannerState *state,
                                     const bool *valid_symbols) {
  if (state->tag_len == 0 || lexer->eof(lexer)) {
    return false;
  }

  // Empty body: the very first line is the terminator.
  if (probe_terminator(lexer, state)) {
    if (valid_symbols[HEREDOC_END] && finish_terminator(lexer)) {
      lexer->mark_end(lexer);
      state->tag_len = 0;
      lexer->result_symbol = HEREDOC_END;
      return true;
    }
    return false;
  }
  if (!valid_symbols[HEREDOC_BODY]) {
    return false;
  }

  bool consumed = false;
  while (!lexer->eof(lexer)) {
    if (probe_terminator(lexer, state)) {
      // mark_end sits at the previous line end — exactly the body end.
      lexer->result_symbol = HEREDOC_BODY;
      return true;
    }
    consume_to_eol(lexer);
    lexer->mark_end(lexer);
    consumed = true;
  }
  // EOF without a terminator line: unterminated body.
  if (consumed) {
    lexer->result_symbol = HEREDOC_BODY;
    return true;
  }
  return false;
}

bool tree_sitter_splicery_external_scanner_scan(
    void *payload, TSLexer *lexer, const bool *valid_symbols) {
  ScannerState *state = payload;
  // START is valid in piece position; BODY/END only inside an open
  // heredoc — never together. A failed START probe may consume bytes,
  // so it must not fall through: returning false rewinds the lexer.
  if (valid_symbols[HEREDOC_START]) {
    return scan_heredoc_start(lexer, state);
  }
  return scan_heredoc_body_or_end(lexer, state, valid_symbols);
}
