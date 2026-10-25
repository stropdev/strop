# 0073 — Documentation language injections

## Design

The existing Markdown pipeline already resolves explicit fenced languages through
one statically linked registry, including non-Rust languages. This change extends
that pipeline to syntax-selected documentation containers; it does not introduce a
new parser, dependency, document copy, or input-to-render analysis path.

Tree-sitter queries select containers and a small `injection.documentation`
property chooses absolute included-range extraction. Parsers continue reading the
same frozen rope. Delimiters and decoration prefixes are excluded from included
ranges, and descendant ranges are intersected with their parent's included ranges.
This is necessary even when the descendant's syntax node spans excluded bytes.

## Host coverage

- Rust: grammar-recognized `///`, `//!`, `/** ... */`, and `/*! ... */` docs.
  Adjacent same-kind line-comment siblings form one document; blank lines,
  ordinary comments, different doc kinds, and declarations end the group.
- JavaScript, TypeScript, TSX, C, C++, Java: `/** ... */` documentation comments,
  selected as comment syntax nodes, not matching text inside strings.
- Go: block comments directly preceding top-level package/type/function/method/
  variable/constant declarations. Ordinary line comments are not included.
- Python: the first plain/raw triple-quoted string expression in a module,
  class body, or function body. Assignments, bytes, formatted strings, and later
  expressions are excluded. Common continuation-line indentation is removed.

This is not universal comment highlighting, a JSDoc/Doxygen/reStructuredText
parser, or evaluation of Python escapes. Documentation is interpreted as Markdown;
explicit fences use any registered language. Unknown fence languages retain
Markdown code styling, including when their content starts with a known shebang.

A `markdown-rustdoc` registry entry reuses the existing Markdown grammar and
queries. Only fences without an info string default to Rust. Explicit languages,
including unknown languages, never silently fall back to Rust. Rustdoc attribute
labels (`rust,ignore`, `no_run`, `compile_fail`, `edition2021`, and recognized
comma-separated combinations) select Rust only in this variant. They are inert
highlighting metadata, never execution instructions. Attribute reference:
https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#attributes

## Isolation and lifecycle

Global `injection.combined` retains its existing Tree-sitter meaning; it is not
used for documentation groups. Rust line-doc groups are expanded along adjacent
syntax siblings even when the requested viewport starts inside a fence. Each
block comment/docstring is an independent parser. Collection remains viewport
selected, cancellation-aware, and subject to the existing eight-level nesting
limit. Incremental edits continue through the owned highlighter tree; a changed
range invalidates only the affected injected parser. No virtual-text offsets are
introduced: every range and highlight remains in the host rope's byte domain.

## Sources and attribution

The Rust doc-comment and rustdoc-default-fence approach is adapted from Helix's
MPL-2.0 queries in `runtime/queries/{rust,markdown-rustdoc}/injections.scm`.
The Go declaration attachment is adapted from `runtime/queries/go/injections.scm`;
Helix explicitly documents its block-comment limitation. Adapted queries retain
attribution and use the existing `queries/LICENSE-HELIX-MPL` license.

Tree-sitter's injection contract:
https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html#language-injection

## Verification

All 53 syntax tests pass, including non-Rust languages, unknown-language fallback,
delimiter/prefix byte domains, independent groups, viewport selection, nested
injections, Python eligibility, and incremental edits versus fresh analysis.
The cell-grid regression exercises Python inside JavaScript documentation,
JavaScript inside Rust documentation, and Go inside Markdown at wide and narrow
viewports. The real headless editor also renders a Rust document containing a
Python fence while exercising paired indentation and trace replay.
