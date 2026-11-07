# Splicery grammar coverage

Grammar-rule ↔ EBNF-family map (spec `splicery.ebnf`, /1) with the
presentation deviations that keep editing stable. A green tree is not
a valid recipe, an exact-match inventory, or any Splicery gate.

| EBNF family | Grammar rules | Notes |
|---|---|---|
| `document` | `document` | one file op or a fragment |
| `file_edit` | `edit_file` | `as bytes` via `raw_profile` |
| `create_file` | `create_file` | |
| `rename_file` | `rename_file` | `to` contextual |
| `remove_file` | `remove_file` | |
| `fragment` | `fragment` | |
| `statement` | `statement` | |
| `binding` | `binding_statement` | `:=` one token |
| `change` | `replace_statement`, `before_statement`, `after_statement`, `delete_statement` | |
| `iteration` | `iteration` | body holds changes only |
| `collection` | `line_matches`, `frozen_collection` | `within`/`expect` contextual |
| `selector`/`reference` | `selector`, `reference` | `.group` suffix |
| `one …` | `one_selector` | `text`/`line`/`hex` spellings contextual to this node |
| `between … and …` | `between_selector` | `and` contextual |
| `range [N, N)` | `range_selector` | |
| `payload`/`[…]` | `payload`, `payload_list` | |
| `piece` STRING | `string` | see deviations |
| `piece` HEREDOC | `heredoc` (`heredoc_start`/`heredoc_body`/`heredoc_end` external scanner) | |
| `piece` HEX | `hex_literal` | any run of hex digits (odd counts refuse upstream) |
| `keep(ref)` | `keep_piece` | reserved spelling, not an identifier |
| `lines like ref […]` | `lines_like_piece` | `like` contextual |
| `indent(STRING, payload)` | `indent_piece` | |
| `HANDLE` | `handle` = `@` + `name` | two tokens so marker/name take different classes |
| `NAME` | `name` | ASCII; reserved words tolerated (upstream refuses) |
| `NAT` | `nat` | `0` or `[1-9][0-9]*`; leading zeros lex as two nats |
| `RX` | `regex` | ONE raw token; `"` ends it, single line |
| comments | `comment` | `#` to line end outside literals |

## Documented presentation deviations

- **Escape superset**: `escape_sequence` accepts `\<any non-newline>`;
  the spec decodes only `\" \\ \n \r \t`. Keeps half-typed strings
  lexable while editing.
- **Optional closing quote**: an unterminated `"` colors as a string to
  end of line; upstream refuses physical newlines in strings.
- **Hex digits**: odd counts and any length are accepted here; they are
  a semantic refusal in Splicery.
- **Tag length**: the scanner enforces the 64-byte tag limit; reserved
  words inside names/groups are not checked (upstream owns that).
- **Isolated CR is whitespace** outside literals; the reference lexer
  rejects it.
- **Reserved words as names**: `@text`, `@expect` etc. parse as handles
  here; the language refuses them.
