# 0039 — Broader static syntax coverage

Status: implemented and container-verified for 0.18.0. CMake
and Markdown are required; remote editing/writes follow the current release as a
separate RW4 implementation and release, not an excuse to delay these languages.

## Evidence and scope

Strop currently detects 13 languages/grammars: Rust, Python, JavaScript,
TypeScript, TSX, Go, C, C++, JSON, Bash, Fish, Lua and SQL. Its comment that TOML
has no compatible published grammar is obsolete.

[Rootle's registry](https://github.com/rootledev/rootle/blob/0be7cbc9512081e231dddef20e09b6cc62be570f/crates/rootle/src/highlight/registry.rs)
and [dependencies](https://github.com/rootledev/rootle/blob/0be7cbc9512081e231dddef20e09b6cc62be570f/crates/rootle/Cargo.toml)
add Java, C#, Ruby, PHP, TOML, YAML, HTML, CSS and Markdown, with a separate
Markdown-inline grammar. The published crate families use tree-sitter 0.25;
`tree-sitter-cmake` 0.7.4 is MIT-licensed and supplies the additional requested
CMake parser. Adopt these maintained, compatible additions rather than assembling
an unreviewed list of unrelated grammar crates.

New queries are vendored from Helix commit
`079a789e8cb08ead67f19e1971a1b7438b37354b`. The local Markdown injection change
includes all inline/table-cell children, preserving emphasis delimiters. This is
an explicit compatibility adaptation, not another query source or runtime fallback.

The resulting detection set is 23 languages, plus Markdown's internal inline
parser. CMake covers `CMakeLists.txt` and `.cmake`; Markdown covers `.md` and
`.markdown`. Preserve native `Path` detection, exact-basename precedence, bounded
shebang detection, and the current curated language set. Reuse one registry for
filenames and injected-language aliases; never infer an injected language from a
local filename on another filesystem.

## Implementation contract

- Grammars remain compiled into the static binary. No runtime downloads, dynamic
  libraries, installation directory or required language-server setup.
- Retain the existing Helix-vendored query convention and its MPL license. Fetch
  and record the new query sources, compose explicit inheritance where needed,
  and compile every query against the pinned grammar version. Missing or invalid
  queries are failures, not pretend highlighting or regex substitutes.
- Markdown needs both block and inline syntax: headings, lists, quotes, links,
  emphasis, strong text and inline/fenced code. Preserve the full inline-node
  range including delimiter children; Rootle's
  [query composition](https://github.com/rootledev/rootle/blob/0be7cbc9512081e231dddef20e09b6cc62be570f/crates/rootle/src/highlight/queries.rs)
  documents why excluding those children loses emphasis. Recognized fence
  languages use the same registry and highlighter. Unknown fence languages keep
  the outer Markdown code treatment, never a guessed grammar.
- Injected parsing uses byte-preserving included ranges over the frozen rope.
  Keep parsing, query compilation, injection discovery and invalidation on the
  display-analysis owner from 0038. Do not materialize the entire document as a
  String on each edit or move parsing back into the renderer.
- Semantic classes/styles must express Markdown structure on both editor panes
  and picker previews. Keep one class-to-style projection; selection, search,
  operator preview and cursor precedence remain unchanged.
- Split the highlighter into focused modules before crossing the source-file
  ceiling: registry, parser/query ownership, injections and behavior tests. Keep
  the public `strop-syntax` boundary coherent, with no old/new parallel APIs.

## Acceptance and release

1. Real CMake and Markdown samples render through the actual editor, including
   a picker preview, narrow panes, Unicode, edits and undo.
2. Semantic regressions cover CMake commands/variables and Markdown inline/fence
   behavior; incremental results agree with a clean parse after structural edits.
   Each added language has a representative query/grammar compatibility oracle.
3. Large-document typing retains the worker/cancellation and revision ownership
   guarantees. No old-revision injection spans can paint a new revision.
4. Docker fmt/clippy/tests, static linking, applicable protocol gates, hosted CI,
   release notes and the public site's language coverage all agree with what
   ships. Publish with the current release; complete RW4 in the next release.

Verification: the actual editor rendered CMake and Markdown; Markdown also ran
through narrow split panes and edit/undo. Semantic regressions cover all added
grammars, inline/fenced Markdown, HTML injections and clean-vs-incremental results.
The release shares 0038's worker ownership and measured long-line path.
