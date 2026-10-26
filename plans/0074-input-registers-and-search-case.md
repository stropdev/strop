# 0074 — Input-field registers and buffer-search case qualifiers

## Contract

A one-line input field reads the same registers as its owning editor. In insert mode, Ctrl-R followed by a register name inserts that register verbatim at the caret; an empty name or `"` reads the unnamed register. Field Normal mode `p`/`P` inserts after/before the character at the caret, with the usual end-of-field behavior. Named registers are supported; `_` is empty. A multiline payload refuses with a visible message, never truncation or a second line. Picker search/replacement updates use the same change notifications as typing.

## Fuzzy path scope and visual comments

Bare Files text already fuzzy-matches the full workspace-relative path. `path:`
remains a case-sensitive literal substring scope. `fpath:` adds an explicit
smart-case fuzzy scope over that same path, including parent directories;
positive values OR, `-fpath:` excludes, and Boolean metadata uses the same
predicate. `fpath:modules/jweb/ parser` therefore scopes the path while `parser`
controls ranking. No leading-slash rewriting or implicit `path:` reinterpretation.

Visual, Visual Line and Visual Block Ctrl-C toggle line comments for the whole
selected line range. Source language metadata supplies `//`, `#` or `--`;
unknown/block-only languages refuse without edits. Existing indentation is
preserved. If every nonblank selected line has the token, all are uncommented;
otherwise every nonblank line is commented. The edit is one undo unit and returns
to Normal. Insert Ctrl-C closes the insert session like Escape. Ctrl-C never
quits the application: `:q` is the keyboard quit command; a GUI close request
is a separate UI action.

The unified workspace Search host retains Ctrl-R for toggling replacement in both its field modes. Its register insertion path is field Normal `p`/`P`; this preserves plan 0053's existing command rather than introducing a modal conflict. Ordinary Files and other picker fields use Ctrl-R `{register}`. Document Insert mode also accepts Ctrl-R `{register}` through the existing literal insertion and undo/replay machinery. The system `+` register remains asynchronous and document-owned; this patch does not let a delayed clipboard read cross into a newly opened field.

## Search case qualifiers

Buffer `/` and `?` search is case-insensitive by default. At the beginning of the query, `case:sensitive` and `case:ignore` are zero-width case directives with the same meaning as Vim's `\C` and `\c`. `case:insensitive` is a readable alias for `case:ignore`. They may be preceded by `\v` or `\m`. A later `\c`/`\C` still controls the compiled query, matching existing behavior. Mid-pattern `case:` is literal, and `case:smart` is not silently accepted in this dialect.

The qualifier is removed only inside the shared `CompiledQuery` compiler, not by the prompt. Therefore the same compiled query drives preview, execution, `n`/`N`, highlights and trace serialization; the original query text remains intact. `*`/`#` whole-word search also folds by default; `\C` restores exact case there.

## Verification

Real-editor regressions cover `yiw` → Files picker → Ctrl-R `"`, named registers, Unicode and multiline refusal, field Normal `p`/`P`, search-prompt paste and the workspace Search Ctrl-R conflict. Buffer-search tests cover insensitive default, ignore/sensitive qualifiers, aliases, Unicode folding, mid-pattern literal text and `n` repeat. File-plan and real-picker tests cover `fpath:`, exclusions and literal `path:` compatibility. Visual tests cover commenting, uncommenting, mixed selections, indentation, CRLF, unknown languages, one-step undo and no Ctrl-C quit. The patch releases as 0.46.1 after the complete repository gate, since 0.46.0 is already published.
