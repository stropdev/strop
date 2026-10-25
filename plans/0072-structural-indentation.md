# 0072 — Structural indentation

## Contract

Enter between paired code delimiters creates an indented inner line and an aligned closing line. A closing delimiter typed after whitespace aligns with its matching opener's leading whitespace, rather than removing one arbitrary indent unit. Each cursor gets its own decision, applied bottom-up in the existing insert transaction. Tabs and the document's newline convention are preserved verbatim. `O` inherits the current line's indentation without treating its trailing opener as a preceding block.

Plain text retains whitespace only. Unnamed scratch buffers retain the existing generic brace workflow; their conservative syntax context is the Rust grammar, without Python colon rules. Known language identity comes from the existing source-path/shebang registry. Strings and comments are not structural delimiters. Unsupported syntax falls back to inherited indentation, never an invented outdent.

Structural delimiter support is explicit for Rust, Python, JavaScript, TypeScript, Go, C, C++, JSON, Java, C#, and CSS. Python alone adds colon continuation when not already inside a delimiter. Other named languages (including Markdown), unknown filenames, and prose files inherit whitespace. This is delimiter indentation, not a language formatter: it does not implement keyword-based `end`/`else` alignment, HTML tag indentation, or injected-language indentation inside comments/fences. Malformed literal regions and mismatched delimiters conservatively retain whitespace. Scratch prose containing balanced braces intentionally shares scratch's generic code behavior; assigning a prose filename selects whitespace-only behavior.


## Reference behavior

- [VS Code language configuration](https://code.visualstudio.com/api/language-extensions/language-configuration-guide): language-specific increase/decrease rules, lexical exclusions, and `indentOutdent` distinguish paired Enter from ordinary continuation.
- [Zed language extensions](https://zed.dev/docs/extensions/languages): syntax `@indent`, `@start`, `@end`, and `@outdent` tie indentation to actual constructs rather than all punctuation.
- [Helix indent queries](https://docs.helix-editor.com/guides/indent.html): the hybrid heuristic preserves existing indentation and adds structural differences; `o` and `O` use different source positions.
- [Neovim indentation](https://neovim.io/doc/user/indent.html): autoindent copies whitespace, while structural methods align a closer with the matching opening construct; these are not interchangeable with deleting one unit.

## Ownership and execution

The editor owns edit plans and cursor landing positions. Syntax owns source classification using its existing Tree-sitter parser, not a second comment/string lexer. Expensive structural decisions use the existing cancellable, revision/cursor-stamped resolution worker and ordered typeahead queue. Input/render never synchronously parses or scans an arbitrary source prefix. Per-document indentation and existing transaction publication remain authoritative.

The inline path is limited to at most 4 KiB of source and 32 cursor probes. It accepts only lexically unambiguous punctuation; encountering quotes, comments, escapes, template/raw-string/regex/markup markers sends the request to the parser owner instead of guessing. The limit is an execution boundary, not a functionality boundary: ordinary code later in a large file receives the same decisions from the worker. Only leading whitespace is copied, never a whole source line.

Resolution jobs carry typed source-document identities, revisions, delimiter anchors, source/view cursor mappings, indentation units, and newline styles. Collection decisions inspect authoritative source buffers, including matching openers outside an excerpt; replacements must stay within their owned excerpt. Publication checks document, pane, revision, cursors, source identity, mode, and indentation settings. A stale decision cancels its ordered continuation rather than editing newer text. Worker parser slots consume the same mutation journal used by display analysis; missed revisions invalidate the tree. Parser retention is capped at sixteen source slots.


Replay records user intent separately from generated indentation: Enter is replayed as Enter at the destination, including paired splitting and CRLF, rather than replaying stale generated spaces. Literal recording remains available for block insertion and completion bookkeeping. Counted insertion, macros, and collection source mapping use the same insert consumer.

Completion acceptance updates both the existing literal recorder and the semantic tape. Generated dot/count keys bypass macro recording so a recorded counted insert does not replay its expansion twice. Backspace deletes a whole Unicode scalar (or CRLF break) at each cursor, keeping recorded prefix replacement and its replay byte-safe. Conflicting overlapping indentation edits are refused explicitly instead of applying overlapping pre-edit ranges.

The tape distinguishes interpreted keys, verbatim literal insertions, and relative snippet-stop metadata. Bracketed paste joins the open Insert undo group and records its bytes without turning payload braces/newlines into indentation commands. Completion literals use the same verbatim replay, while snippet selection replacement and consumed Tab navigation replay through the existing snippet consumer. Insert-mode Ctrl-R is currently a no-op in the editor; this change does not fabricate register-insertion support.

Tree-sitter error recovery may hide unterminated raw-string start tokens or retain standalone literal fragments such as Python's `string_start`, then parse literal braces as sibling code. The worker treats uncovered quotation bytes and orphaned start/content/end fragments in an ERROR node as uncertain literal extent through EOF. Only complete, error-free outer string/comment nodes skip their internals. This protects malformed raw/byte/C-string and multiline Python input without changing the grammar or implementing a competing lexer.


## Regression obligations

Paired Enter (the observed failing `fn main() {}` case), matching-opener alignment from over/underindentation, nested/mismatched delimiters, comments and strings, Unicode before an opener, plain text punctuation, Python-only colons, `O`, tab/CRLF preservation, independently indented multiple cursors, one-step undo, dot/count replay, large-file deferred input and stale delivery, and collection/macro consumers.

Consumer coverage lives in `editor/tests/smartindent_tests.rs`, retaining the existing scratch-buffer regressions and adding source-aware deferred cases. All 29 focused indentation regressions pass, including malformed literals and read-only transitions. The real headless editor has exercised paired Enter, undo/redo and full-content trace replay; renderer, PTY, differential and UI-protocol suites also pass. Workspace Clippy passes with warnings denied.
