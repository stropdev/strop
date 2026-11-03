# 0078 — Paper and Paper Dark theme handoff

Status: **implemented and locally qualified** (2026-11-03).

## 1. User intent and scope

Tarek likes the new **Paper** theme on his personal website and wants it in
Strop. He subsequently requested a matching dark variant with the same clean,
restrained character. This session updates the handover for another agent;
it does not implement or release either editor theme.

The checked editor registry still has six themes and neither Paper variant.
Implement this pair as two additional compiled-in choices:

| Canonical name | Display label | Character |
| --- | --- | --- |
| `paper` | Paper | Light: warm off-white paper, deep green ink, forest-green accents |
| `paper-dark` | Paper Dark | Dark: warm green-black paper, soft ivory ink, muted sage accents |

Retain `paper` as the light variant's canonical name from the original handover;
do not add a `paper-light` alias. These are not renames of `light`, `dark`, or
`strop-ink`; existing palettes and their identities remain unchanged.

Keep `dark` as the zero-config default. Do not change anyone's configuration,
add automatic light/dark switching, redesign editor layouts or fonts, import
all the website's other palettes, or introduce another theme subsystem.
Website changes, version bumps, tagging, and publication are outside this
handover's implementation scope.

Read [AGENTS.md](../AGENTS.md) before implementation. Paths below are relative
to the Strop repository root unless stated otherwise.

## 2. Authoritative visual source

- Local sibling: `../tknawara.github.io/assets/site.css`, the opening `:root`
  block. The personal-site repository is the donor, not Strop's landing page.
- Pinned source: [profile CSS at aeb7aa2](https://github.com/tknawara/tknawara.github.io/blob/aeb7aa2/assets/site.css#L1-L20).
- Visual reference: [tknawara.dev](https://tknawara.dev/), selecting **Paper**
  in its footer if a different palette is saved. The live site can change;
  the pinned CSS and values below identify the requested version.

Writfold inspired the profile's visual direction. Port the profile's actual
Paper palette, not an approximation taken from another website or a screenshot.

### Paper: exact light source colors and recommended editor mapping

| Source token | sRGB hex | Editor use / constraint |
| --- | --- | --- |
| `--bg` | `#f8f8f3` | `base`; preserve the paper, not pure white |
| `--card` | `#fdfdf9` | `surface`; subtly lighter panels and pickers |
| `--text` | `#20362d` | `text`; deep green, not neutral black |
| `--dim` | `#475951` | `muted`; readable comments and subdued labels |
| `--accent` | `#1a6650` | `accent`; the site's actual mixed foreground accent |
| `--blue` | `#18765b` | Raw forest-green accent; candidate function/type ink |
| `--green` | `#526e38` | Moss green; candidate string/addition ink |
| `--peach` | `#986139` | Earthy brown; candidate number/constant ink |
| `--mauve` | `#78618e` | Muted violet; candidate keyword/attribute ink |
| `--border` | `#dce1d6` | Decorative guides and quiet separators, never comment ink |
| `--border-strong` | `#a9b8a8` | Stronger borders; still not ordinary text ink |

The two mixed colors above are nearest-8-bit sRGB resolutions of:

- `--dim`: 82% `--text` + 18% `--bg`.
- `--accent`: 75% `--blue` + 25% `--text`.

These mixtures were calculated for this handover. Strop's existing const
`blend(base, overlay, percent)` truncates channels rather than rounding, and
its percentage refers to the **second** argument. Use the resolved literals
for these two identity colors rather than accidentally reversing the mix.
The website's legacy token name `--blue` is green; do not blindly map CSS
variable names onto ANSI color indices.

### Paper: editor-only roles to design and verify

The website is not a complete editor palette. Fill **every existing `Theme`
field**, including all 16 ANSI entries, rather than inheriting dark defaults.
Use [the original palette module](../crates/strop-core/src/theme/ink.rs) and
[the built-in light palette](../crates/strop-core/src/theme/builtin.rs) as
construction patterns, not as replacement visual identities.

- Statusline, current row, and inactive panes: restrained paper/green-grey
  variations. Keep borders quiet and the content visually dominant.
- `secondary`: an ink between `text` and `muted`, not a decorative pale grey.
- Selection, matching pairs, operator previews, and substitution flashes:
  pale green washes with distinct strengths and readable syntax. The website
  inverts selected text; do **not** copy that dark selection background into
  editor surfaces whose foregrounds are not also inverted.
- Syntax: start with forest-green functions, moss strings, brown numbers,
  violet keywords, and subdued green-grey operators/punctuation. Keep types
  and attributes distinguishable without turning the whole buffer green.
- Error/deletion, warning, and information roles need additional semantic
  hues: restrained rust/red, ochre, and blue/teal respectively. They are new
  editor adaptations, not colors claimed to exist in the source website.
- Diff row washes and their stronger changed-span variants must retain the
  distinction between addition and deletion without overwhelming code.
- ANSI-16: design a coherent light-terminal palette with recognizably
  distinct red/green/yellow/blue/magenta/cyan roles. Preserve the conventional
  ANSI-256 cube and greyscale ramp at indices 16–255. Child-selected truecolor
  and OSC overrides must retain their existing ownership.

Preserve the source base, text, and accent identity. Tune editor-only washes
and semantic additions against actual rendered cells; do not substitute
barely visible highlights just to make the page's restrained look literal.

### Paper Dark: matching dark variant

This is an original companion design, not an inverted light palette or a
relabelled Strop Ink. Keep the same forest/moss/brown/violet relationships,
but use warm green-black surfaces and soft ivory text. No pure-black canvas,
glowing neon accents, gradients, transparency-dependent text, or extra chrome.
Selection and change washes should be visible without competing with code.

The table supplies every scalar field of the current `Theme`; repeated-role
rows deliberately share an ink. Use static `hex(...)` initializers in the
same palette module as the light variant.

| `Theme` role(s) | sRGB hex | Purpose |
| --- | --- | --- |
| `base` | `#151d19` | Warm green-black canvas |
| `surface` | `#1c2720` | Quiet elevated panels and pickers |
| `statusline_bg` | `#111914` | Recessed modeline |
| `cursor_row_bg` | `#202d24` | Current row without a bright stripe |
| `inactive_bg` | `#17201b` | Inactive pane surface |
| `border` | `#53695b` | Visible structural boundaries |
| `guide` | `#35473b` | Subdued decorative indent guides |
| `text` | `#e3e9dd` | Soft ivory with a slight green cast |
| `muted`, `class_punctuation` | `#9daa9e` | Readable comments and quiet syntax |
| `secondary`, `class_operator` | `#bfccbe` | Useful secondary context |
| `accent`, `class_function` | `#9bc7ac` | Sage-green emphasis |
| `class_keyword` | `#b7a6c9` | Dusty violet |
| `class_type`, `diag_info` | `#9abac5` | Muted blue-slate |
| `class_string` | `#a8bb87` | Moss ink |
| `class_number` | `#cea989` | Warm brown/tan ink |
| `class_attribute`, `warning` | `#d3b477` | Restrained ochre |
| `diag_error` | `#df9992` | Warm rust/coral, distinct from normal syntax |
| `select_bg` | `#2c4637` | Green selection wash |
| `pair_bg` | `#385440` | Matching pair, stronger than selection |
| `preview_bg` | `#263e30` | Quiet operator-pending preview |
| `flash_bg` | `#36533f` | Stronger substitution/change flash |
| `diff_add_bg` | `#203b2b` | Addition row |
| `diff_add_strong_bg` | `#304f38` | Changed span within an addition |
| `diff_del_bg` | `#3b2927` | Deletion row |
| `diff_del_strong_bg` | `#573732` | Changed span within a deletion |

Use this companion ANSI-16 set in standard index order. Bright colors are
modestly lifted, not fluorescent. These are terminal defaults, not a promise
that arbitrary child-selected foreground/background combinations are readable.

| Index | ANSI role | sRGB hex |
| --- | --- | --- |
| 0 | Black | `#121a15` |
| 1 | Red | `#cf8c85` |
| 2 | Green | `#9aaf7d` |
| 3 | Yellow | `#c3a66c` |
| 4 | Blue | `#8bacb9` |
| 5 | Magenta | `#a994bd` |
| 6 | Cyan | `#8fb8ab` |
| 7 | White | `#d5ddcf` |
| 8 | Bright black | `#748b79` |
| 9 | Bright red | `#e6a9a2` |
| 10 | Bright green | `#bccd9a` |
| 11 | Bright yellow | `#dfc58e` |
| 12 | Bright blue | `#b0cbd4` |
| 13 | Bright magenta | `#c9b6d9` |
| 14 | Bright cyan | `#adcfbf` |
| 15 | Bright white | `#e3e9dd` |

Calculated sRGB contrast checks for this proposed dark palette:

| Check | Result |
| --- | --- |
| Main text on all 13 editor background roles | Minimum **6.76:1** |
| Syntax, comments, secondary/accent and diagnostic ink on base and surface | Minimum **6.38:1** |
| Selection against base | **1.67:1** |
| Pair above selection; strong preview/diff above weak variants | Pass |

These are calculations over the proposed colors, **not editor rendering
evidence**. Preserve this identity while adjusting only as necessary after
real TUI/GUI inspection. In particular, inspect syntax inside selected and
diff-highlighted text, not just plain source on the base surface.


## 3. Implementation map

The current registry contains `dark`, `light`, `catppuccin-mocha`,
`catppuccin-latte`, `nord`, and `strop-ink`. The editor already propagates a
registered palette through config, CLI selection, the session picker, syntax,
chrome, and terminal defaults.

| File | Work / existing contract |
| --- | --- |
| `crates/strop-core/src/theme/paper.rs` **(new)** | Two complete static constants, `PAPER: Theme` and `PAPER_DARK: Theme`; follow the existing small palette modules. No runtime parsing, downloads, dependencies, or per-cell work. |
| [crates/strop-core/src/theme.rs](../crates/strop-core/src/theme.rs) | Declare/re-export the module and both constants; append `ThemeId::Paper` and `ThemeId::PaperDark`; extend the current `ALL` from six to eight; add `name`, `label`, `description`, and `palette` arms; update both atomic decoding cases in `current_id()`. |
| [crates/strop-engine/src/editor/theme.rs](../crates/strop-engine/src/editor/theme.rs) | Existing picker and `:theme NAME` path use the registry. Reuse `apply_theme`; it already queues terminal refreshes. No second selection path. |
| [crates/strop/src/cli.rs](../crates/strop/src/cli.rs) | `--theme` parsing and invalid-choice help already use `ThemeId::from_name` / `ALL`; verify propagation rather than adding a hardcoded Paper branch. |
| [crates/strop-engine/src/config.rs](../crates/strop-engine/src/config.rs) | Config stores the serde `ThemeId`. Keep the default `Dark`, existing precedence, and canonical persisted names. |
| [crates/strop-terminal/src/model/mod.rs](../crates/strop-terminal/src/model/mod.rs) | `Palette::for_theme` already derives terminal foreground, background, and ANSI-256 from the core palette. Do not duplicate either Paper variant here. |
| [crates/strop-terminal/src/service/mod.rs](../crates/strop-terminal/src/service/mod.rs) and [worker.rs](../crates/strop-terminal/src/service/worker.rs) | Existing asynchronous theme-request encoding uses `ALL` indices. Preserve this ownership and the nonblocking input path. |
| [crates/strop-engine/src/editor/trace/seed.rs](../crates/strop-engine/src/editor/trace/seed.rs) | The effective theme is part of replay state; verify both variants are retained through the existing serialized path. |

Before editing exported symbols, use language-server references if available
and inspect any additional exhaustive matches in the current checkout.

**Easy-to-miss decoder:** `ThemeId` is `#[repr(u8)]`, and `current_id()` has
an explicit byte-to-variant match. Append both variants without renumbering
existing entries, map both values there, and leave `CURRENT` initialized to
zero. Adding only enum/registry entries otherwise lets a selected Paper
variant fall back to Dark when read through the live selector.

### Selection behavior to preserve

- `strop --theme paper ...` and `strop --theme paper-dark ...` override the
  configured theme for that launch.
- `theme = "paper"` or `theme = "paper-dark"` in the normal user config
  persists the corresponding startup choice.
- `:theme paper`, `:theme paper-dark`, and selecting either entry in `:theme`
  change the session only; none rewrites config.
- Cancelling the picker or submitting an invalid name leaves the active
  theme unchanged. Reuse the existing tolerant CLI/session name lookup and
  canonical serde spelling; do not add aliases.
- Existing and newly created terminals receive the selected defaults,
  including idle and inspection views, without losing child overrides,
  pinned text, input acknowledgements, or replay ownership.

## 4. Tests and a known registry-order trap

Reuse the existing behavioral suites; most already iterate `ThemeId::ALL`:

- [Core theme tests](../crates/strop-core/src/theme/tests.rs): text contrast,
  syntax/comment contrast, overlay strength, ANSI-256 semantics, serialized
  names, and isolated-process live selection. Extend the canonical-name
  contract with `paper` and `paper-dark`; do not weaken thresholds for either.
- [Real child-editor theme tests](../crates/strop/tests/themes.rs): rendered
  config/CLI precedence, session-only switching, cancellation, invalid names,
  and replay. Include a journey that selects both variants through the picker,
  switches between them, and replays the result, not just CLI parse checks.
- [Rendered-surface tests](../crates/strop/src/render/theme_tests.rs): source,
  picker, diffs, help, and statusline with the registered palettes.
- [Native terminal tests](../crates/strop-terminal/src/vt/tests.rs): every
  registered theme initializes and rethemes a real terminal; explicit RGB
  and OSC ownership/reset behavior already have dedicated coverage.
- [Engine terminal theme tests](../crates/strop-engine/src/editor/terminal/theme_tests.rs):
  idle/startup/inspection updates and input-sequence ownership. Exercise
  dark-to-Paper, Paper-to-Paper-Dark, and reverse transitions.

**Known trap:**
[service/tests.rs](../crates/strop-terminal/src/service/tests.rs),
`theme_requests_coalesce_and_wake_even_when_child_input_is_full`, queues all
registered themes and then hardcodes `StropInk` as the last result. Adding
the pair invalidates that incidental registry-order assumption. Remove that
assumption rather than re-pinning it to Paper Dark: use an explicit request
sequence and prove **latest requested theme wins while the child-input queue
is saturated**, with the existing wake/budget assertions retained.

Avoid tests that only duplicate hex literals, count themes, inspect source
text, or check a successful command exit. Keep global selector mutations
isolated in child processes, as the existing suites do.

## 5. Acceptance and verification

### Contrast and appearance

- Main text must meet the existing **4.5:1** floor on every editor background:
  base, panels, statusline, current/inactive rows, selections, pairs, previews,
  flashes, and both strengths of diff washes.
- Aim for **4.5:1** for both variants' ordinary syntax, comments, and small UI labels
  on their actual surfaces. At minimum preserve the existing **3:1**
  syntax/comment contract on base and popup surfaces. Do not count decorative
  border/guide colors as text or claim arbitrary child terminal colors pass.
- Existing overlay contracts remain: selection/base contrast at least
  **1.15:1**, matching-pair emphasis above selection, and strong diff/flash
  overlays stronger than their weak counterparts.
- Inspect Rust, TypeScript, and Markdown with comments and fenced code,
  search matches, visual selection, operator-pending preview, matching pairs,
  diagnostics, completion, pickers, Git diffs, help, and modelines. Verify
  readable text and unambiguous state, not only plausible color constants.

### Runtime proof

After implementation, launch the freshly built editor separately with
`--theme paper` and `--theme paper-dark`. Exercise both direct commands,
picker selection/cancellation, switching between the pair, and returning to
an existing theme. Capture both variants at a normal and a small TUI size
(the existing theme work used 90×24 and 42×16). Use isolated HOME/XDG paths
for config and replay checks; do not alter Tarek's actual preferences.

Inspect an embedded terminal before and after live switches between both
variants, including an idle child and Normal-mode inspection. Confirm defaults
change, explicit RGB/OSC overrides survive, and OSC resets resolve to the
selected variant. Check the GUI's shared rendered surfaces when that runtime
is available; state unexercised platforms rather than claiming visual proof.

Run focused suites after the code is complete (host Cargo is an optional
iteration path under AGENTS.md), then the repository's required full gate:

```sh
cargo test --locked -p strop-core theme::tests
cargo test --locked -p strop-editor --test themes
cargo test --locked -p strop-editor render::theme_tests
cargo test --locked -p strop-terminal theme
cargo test --locked -p strop-engine theme
docker compose run --build --rm test
```

Test output alone is not the visual smoke. Record the actual launch,
interactions, screenshots/cell captures, and any limitations. No release is
requested; if separately authorized, follow all release gates in AGENTS.md.

## 6. Documentation and completion

With the implementation, update [docs/editor.md](../docs/editor.md) with both
entries and config/CLI/session examples for `paper` and `paper-dark`, add an
appropriate [CHANGELOG.md](../CHANGELOG.md) entry, and add a follow-on reference
from [0005 §9](0005-configuration.md#9-curated-built-in-themes-owner-authorized-2026-10-19).
Preserve the historical six-theme evidence rather than rewriting it as
eight-theme verification. Update this plan's status only after both variants
and the stated checks actually pass.

This handover is grounded in the current source, existing tests, and the
profile's pinned light CSS. Light color mixtures and the proposed dark
palette's contrast/overlay relationships were calculated. **No Paper or
Paper Dark implementation, Strop build, editor runtime smoke, or native test
run was performed while writing this document.** Light editor-only roles and
actual rendering evidence for both variants remain the implementing agent's work.

## 7. Implementation evidence

The two palettes and registry paths landed with the existing config, CLI,
picker, replay and terminal owners. Core contrast/registry tests, real child
picker/config/CLI/replay journeys, rendered-surface tests, native terminal
tests and engine transition tests passed in the full
`docker compose run --build --rm test` gate (format, Clippy and workspace
tests). Release assurance pins and the catalog/install fixtures also passed.

A freshly built debug editor was launched separately with `--theme paper` and
`--theme paper-dark`. Real TUI captures at 90×24 and 42×16 contain the exact
Paper/Paper Dark RGB roles for Rust and TypeScript syntax and chrome. Headless
captures exercised Rust, TypeScript and Markdown/fenced syntax, search,
selection, matching pairs, picker, help, modeline and embedded-terminal
inspection; the generic rendered suite covers diagnostics, completion and diff
surfaces across every registered palette. Direct commands, picker
accept/cancel, pair switching and return to Dark were exercised with isolated
HOME/XDG paths.

A real embedded shell emitted truecolor and OSC palette overrides before
Dark/Paper/Paper Dark transitions. Captures preserved `#010203` text and the
child-selected `#123456` ANSI red and `#345678` background through a live
switch; OSC 104/110/111 resets then resolved to Paper while explicit truecolor
remained. GUI surfaces were not exercised in this Linux/WSL TUI runtime.
