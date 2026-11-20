---
rfc: "2026-11-20-project-indentation-policy"
title: "Project policy for access-label indentation"
status: "draft"
implementation: "not-started"
authors: []
created: "2026-11-20"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/180"
milestone: "Later"
supersedes: []
superseded_by: []
blocked_on: ["Should Strop adopt nearest-project editor configuration and a typing-time access-label profile while preserving Vim defaults?"]
legacy_id: "0084"
---

# RFC: Project policy for access-label indentation

This untracked handover proposes a per-project typing policy, not the already shipped v0.51.2 electric C-family label fixes. The editor config still has only Default and User layers (crates/strop-engine/src/config.rs); current indentation follows Vim, and clangd formatting on save does not select a typing-time profile.

[Tracking issue #180](https://github.com/stropdev/strop/issues/180).

Follow-up issues: [#73](https://github.com/stropdev/strop/issues/73).

Imported from the working tree; no earlier Git revision or author is known. The date is the import date.

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: proposed, not implemented. This records the 0.51.2 access-label
research and the preferred future design. It does not change the vim defaults
or promise clang-format compatibility.

## Current behavior and decision

Electric `:` intentionally follows vim. For C/C++ access specifiers,
`public:`/`protected:`/`private:` land one document indent unit inside the
enclosing block (`cino-g`), and their body lands one further unit inside the
label (`cino-h`). A document using two-space indentation therefore produces:

```cpp
class F {
  public:
    int value;
};
```

LLVM style instead commonly places the label at the enclosing block margin:

```cpp
class F {
public:
  int value;
};
```

Both are established styles. Helix documents the second convention; clang-format
stock styles disagree; vim defaults to the first. Strop's vim-fidelity doctrine
means the default must remain the first. The LeetCode project's explicit
`.clang-format` now applies `AccessModifierOffset: -2` and `IndentWidth: 2`
through clangd on save. That is a save-time workaround, not evidence that strop's
typing-time policy reads clang-format.

Relevant existing contracts:

- [0005](2026-09-01-configuration.md) already chooses data-only TOML layering:
  embedded defaults < user `config.toml` < nearest project `.strop.toml`.
  Project editor-knob loading is still an implementation gap in the current
  `ConfigLayer`, not a missing design precedent.
- [0012](2026-09-03-project-config.md) keeps `.strop/languages.toml` separate for LSP
  configuration. Do not put indentation policy there.
- [0072](2026-10-25-structural-indentation.md) owns the pure, per-cursor structural
  planner and its inline/deferred execution paths.

## Recommended direction

Implement the already-planned project editor-config layer, then add a small
typed indentation policy to the existing document indentation settings. Do not
invent `editorconfig.json`, a second project-file convention, or a formatting
expression language.

An initial policy needs only named, observable profiles, for example:

```toml
[indentation]
access_label = "block_margin" # embedded default: "vim"
```

- `vim`: label = enclosing block indentation + one unit; body = label + one
  unit. This remains the default everywhere.
- `block_margin`: label = enclosing block indentation; body = label + one unit.
  This covers the requested LLVM-like layout without claiming general
  clang-format equivalence.

The profile describes a structural relationship, not a C++ command or a raw
regex. The language registry still decides which syntax node/keyword is an
access label; policy only decides where that recognized label lands. Other
labels (`case`, `default`, named labels), `std::`, strings/comments and
non-C-family languages keep their existing rules. Add another profile or a
more general anchor+offset representation only when a real third style needs
it; arbitrary numeric knobs are not the first implementation.

## Where other formats fit

### `.editorconfig`

The correct filename is `.editorconfig`, using INI-style sections, not JSON.
Its standard properties cover generic `indent_style` and `indent_size`, but not
C++ access-label anchoring. It can eventually feed the existing per-document
unit/style resolution. It cannot express the policy in this document, so it is
not the whole solution.

If adopted, follow the [EditorConfig specification](https://spec.editorconfig.org/)
for discovery, glob sections, `root = true`, precedence and unsupported keys.
Do not add an unofficial access-label property and call it EditorConfig.

### `.clang-format`

Do not ship a line-oriented parser for `AccessModifierOffset`. Real files also
support `BasedOnStyle`, multiple YAML documents and `Language:` blocks, nested
configuration and tool-version differences. A small parser would silently
produce plausible but wrong indentation.

Runtime clang-format support is optional and separate from the core policy. If
there is sufficient demand, either:

1. run `clang-format -dump-config` as a bounded background project-load job and
   cache the typed result, with explicit fallback when the tool is absent; or
2. provide an explicit import that writes `.strop.toml`, leaving normal editing
   independent of C++ tooling.

Never spawn clang-format on a keystroke. The zero-config binary must not require
it, and non-C++ projects must not depend on it. LSP format-on-save remains a
separate operation, not a substitute for the typing-time policy.

Reference: [clang-format style semantics](https://clang.llvm.org/docs/ClangFormatStyleOptions.html#accessmodifieroffset).

## Resolution and provenance

Use one typed resolved value per document, carried in the existing structural
`Request` to inline and worker planners. Recommended precedence, later wins:

1. embedded vim defaults;
2. user `config.toml`;
3. generic `.editorconfig` properties, if that adapter is implemented;
4. nearest project `.strop.toml`;
5. explicit session/document overrides such as `:tab-size`.

Only the layers that define a property participate in that property's chain.
For example, `.editorconfig` may choose width without changing `access_label`.
Unknown or invalid policy values reject the offending layer and retain the last
good configuration, following 0005. `:explain` must show the effective profile
and its winning source. Project files remain data-only and gain no executable
or trust-bypass capability.

Config discovery, reads and watchers belong to jobs at project/document load or
change notification. Remote buffers resolve configuration in their own workspace
namespace through the existing resource/worker authority; never read the host's
project file for an SSH or container document. Publication must reject stale
policy revisions just as it rejects stale syntax/indent decisions today.

## Adoption slices

1. Implement 0005's project editor-config discovery/layering with provenance and
   last-good failure behavior, local and remote.
2. Add the typed access-label profile to document settings and `Request`, wire
   the shared planner, and preserve `vim` as the byte-for-byte default.
3. Add `.editorconfig` only for generic unit/style properties if needed.
4. Consider explicit clang-format import or background dump-config only after
   the language-neutral policy is stable.

The first two slices solve the motivating project without either external
format. The others are optional adapters, not prerequisites.

## Acceptance evidence

- Existing vim regressions remain unchanged with zero configuration.
- `block_margin` works in `.cpp` and C-mapped `.h`, nested classes, inline and
  deferred parsing, multiple cursors, undo/dot/macros, tabs and CRLF.
- `case`/`default`, named labels, `std::`, literals and other languages do not
  change behavior.
- Project/user/session precedence, invalid-layer recovery and `:explain`
  provenance are observable.
- Local, SSH and container documents use only their own workspace policy.
- No config read, YAML parse, process spawn or network wait occurs in
  input→render. Stale policy updates cannot edit newer text.
- Save-time formatting and typing-time policy are tested independently; no test
  claims they agree unless the project explicitly configures both.

## Non-goals

- A full C++ formatter or complete clang-format emulation.
- Changing vim's default access-label placement.
- A new JSON configuration format or an EditorConfig vendor extension.
- Inferring semantic style heuristically from nearby lines.
- Tying the core editor or its release artifact to clang/clangd.
