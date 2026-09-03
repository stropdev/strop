# 0012 — Project-level LSP config: helix-style languages.toml

> Status: decided + implemented (0.2.2). Reconciles the filename
> disagreement between 0005 (`.strop.toml`) and 0009 (`strop.toml`) for
> the LSP surface, and delivers the user ask: bring your helix
> `languages.toml`.

## Problem

0009 §2.3 parked server overrides in "config (`strop.toml`/0005)" without
a shape; 0005's layering names `<project>/.strop.toml` for editor knobs
and never designed an LSP section. Meanwhile helix users already have a
`languages.toml` worth reusing: `[language-server.NAME]` definitions with
an arbitrary `config` table, and per-language server lists.

## Decisions

1. **Own file, own layering — no `[lsp]` table inside config.toml.**
   LSP server config is `languages.toml`, parsed and owned by strop-lsp:
   ```
   embedded registry  <  $XDG_CONFIG_HOME/strop/languages.toml  <  <project>/.strop/languages.toml
   ```
   `<project>` is the nearest ancestor of the buffer holding
   `.strop/languages.toml`. This resolves the 0005/0009 name split for
   LSP: the project layer is `.strop/languages.toml`; 0005's config.toml
   layering (editor knobs) is unchanged and orthogonal.
2. **Helix-compatible shape, minimal keys.** `[language-server.NAME]`
   with `command`, `args`, and `[language-server.NAME.config]` (an
   arbitrary table, passed through as `initializationOptions`).
   `[language.LANG]` with `language-servers = [...]` overrides which
   server a language uses. Every server-def field is optional — an entry
   naming a registry server *refines* it (set only what changes; command
   and args inherit from the embedded spec).
3. **Divergences from helix, deliberate.** Helix's `[[language]]`
   array-of-tables is not accepted — `[language.NAME]` maps key the
   language directly, so pasted helix headers need the table line
   rewritten. Unknown keys are *ignored*, not rejected: pasting a helix
   entry carrying `scope`/`roots`/`file-types` must keep working, which
   0005 §2's reject-unknown strictness would break. Strictness stays
   for config.toml.
4. **Resolution.** The first resolvable entry of a language's
   `language-servers` wins — strop runs one server per workspace root
   (0009's wave model; multi-server is future work). Config may define
   new servers; they are reachable only through a language override.
   The layer merge is per key: a project entry replaces the XDG entry
   for the same server or language name; everything the other layer
   configured for other keys survives. An override that resolves to
   nothing falls back to the embedded default (load already warned —
   losing LSP over a typo would be worse).
5. **A config-provided command may be absolute** (nix store paths,
   virtualenv binaries): the PATH probe is skipped for absolute
   commands — the spawn itself is the existence check.
6. **Workspace root at attach:** the project layer's directory, else
   the git walk from the buffer (0009's root rule), else cwd.
7. **Failure behavior** follows 0005 §2: a layer that fails to parse is
   rejected with a surfaced warning; a server def with no `command`
   that names no registry server is dropped with a warning; unknown
   names in a language list warn. Nothing bricks — the embedded
   registry keeps resolving, and warnings surface on the statusline at
   attach.
8. **Capability gating (0009 §2.5) is enforced in the client.** The
   Initialize result's server capabilities are stored on the client;
   hover and goto-definition are quiet no-ops when the server doesn't
   advertise them. Capabilities not yet arrived count as unsupported —
   requests never race server startup, and there is no error spam.
9. **No hot-reload for languages.toml yet.** Reload means client
   restarts; deferred until there is a story for that (0005 hot-reload
   covers config.toml). The merged layers load once per process at
   first attach — off the input path, and one project layer per session
   matches the one-server-per-workspace model.

## Files

- `crates/strop-lsp/src/languages.rs` — languages.toml layers: parse,
  merge, warnings, XDG/project discovery.
- `crates/strop-lsp/src/registry.rs` — `ServerSpec<'a>` borrowed from
  the merged tables; the embedded registry behind a `LazyLock`;
  resolution through the layers.
- `crates/strop-lsp/src/lib.rs` — `initializationOptions` passthrough,
  `ServerCaps` gating of hover/goto-definition.
- `crates/strop/src/editor/lsp.rs` — merged config loaded once at first
  attach; absolute commands skip the PATH probe; workspace-root
  precedence; warnings surfaced on the statusline.

## Explicitly out

- Multiple servers per language (helix runs them all; strop takes the
  first until multi-server exists).
- `file-types`/`scope`/`roots` keys doing anything, helix `[[language]]`
  arrays, hot reload, and a per-project trust prompt (config stays
  data-only — 0005 §8).
