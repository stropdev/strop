# 0080 — TUI hierarchy and density: polish handoff

Status: **handoff only; UI changes not implemented by this document**.

## 1. Intent and boundary

Tarek requested a handover for four concrete TUI improvements identified while
reviewing Strop 0.47.2. The goal is clearer hierarchy and better use of terminal
cells, not another editor redesign.

1. A more useful theme picker.
2. More space for meaningful file previews.
3. Consistent pane identities and quieter boundaries.
4. A compact statusline without hiding service failures.

Read [AGENTS.md](../AGENTS.md) first. Preserve Vim grammar, preview/execute
agreement, input ownership, asynchronous service boundaries, and real-buffer
inspection. No animation, new permanent bars, font-dependent icons, new theme
framework, automatic theme switching, or broad settings expansion.

Palette definitions belong to [0078](0078-paper-theme-handoff.md), not this
handover. **The current working source already includes `Paper` and
`PaperDark` in `ThemeId`, with labels and descriptions.** Integrate with the
current registry; do not re-add variants, hardcode its size, reset defaults,
or overwrite another agent's palette work. This source observation does not
claim those changes have been released or qualified.

Prior [0068 UI polish](0068-full-ui-polish.md) and
[0071 quiet diagnostics](0071-correctness-sweep-and-quiet-ui.md) are already
implemented. Keep their cell-safe clipping, padded cards, restrained accents,
and non-underlined diagnostic presentation. Improve the remaining cases
rather than reopen that entire workstream.

## 2. Evidence and limits

The earlier review ran the **installed Strop 0.47.2 binary**, using its actual
headless editor/render path with isolated HOME/XDG directories, a small Rust
file and a Markdown file. It captured normal editing, splits, Files, an
operator search preview, command entry and Theme, at 100×30 and 48×16.
The styled cell grids were inspected as images as well as text.

| Observation | Why it matters |
| --- | --- |
| At 48 columns, theme rows such as `catppuccin-mocha — Pastel dark with a m…` clipped their descriptions. The applied-theme marker is appended after that description. | Filtering, selection and the currently applied palette need distinct, visible identities. |
| At 100 columns, a single short `sample.rs` result occupied a wide results column while the source preview clipped the Rust declaration. | Empty list space was winning over useful code evidence. |
| Two local pane headers showed `sample.rs` beside an absolute temporary-directory path to `README.md`; adjacent vertical rules appeared doubled. | Identity presentation varies with the opening route, and boundary chrome consumes attention. |
| A language-server failure produced a long `lsp: rust-analyzer failed — ...` modeline message, displacing optional context. | Important failures should remain visible without monopolizing the work surface. |

The LSP failure is evidence about **presentation**, not a diagnosis of a
Rust Analyzer defect. Temporary fixtures/captures were removed after review;
there is no retained screenshot directory to rely on. Recreate the scenarios
against the implementing agent's checkout, which may have newer behavior.
The source pointers below were checked while writing this handover; no new
editor build or test run was performed for the document itself.

## 3. Workstream A — theme picker

### Direction

Use the existing modal picker and typed `Payload::ThemeChoice`. Show compact
human-readable labels, a small palette swatch strip, and a visible applied-theme
indicator. Put the selected theme's full description and canonical name in a
bounded detail area, rather than repeating a clipped paragraph on every row.

- Obtain names, labels, descriptions and swatch colors from `ThemeId` and its
  semantic `Theme`; do not maintain a second palette list or copy RGB values
  into the renderer.
- Distinguish the **selected row** from the **applied theme**. The applied marker
  must remain visible when descriptions are omitted or clipped. Do not rely
  on color alone.
- Search must still find canonical names such as `catppuccin-mocha` and
  `paper-dark`, as well as human-readable labels. Keep displayed match spans
  accurate: indices from a hidden canonical-name/description string cannot
  simply be painted onto a differently formatted label.
- Enter remains the application boundary. Moving or filtering the list does
  not silently change the editor's palette. Cancellation leaves the current
  palette unchanged; accepting remains session-only and does not rewrite config.
- Swatches describe a palette, not a second interactive preview renderer.
  Gracefully reduce or omit decoration at tiny sizes before sacrificing the
  name, current indicator, selection or input field.

### Starting points

- [editor/theme.rs](../crates/strop-engine/src/editor/theme.rs):
  `open_theme_picker` currently builds `name — description — current` text;
  `apply_theme` is the single mutation/terminal-refresh path.
- [core theme registry](../crates/strop-core/src/theme.rs):
  `ThemeId::ALL`, `name`, `label`, `description`, `palette`, `current_id`.
- [picker model](../crates/strop-picker/src/lib.rs): `Item`, `Payload::ThemeChoice`,
  `Kind::Theme`; preserve its ranking and typed acceptance contracts.
- [picker rendering](../crates/strop/src/render/picker/mod.rs) and
  [result rows](../crates/strop/src/render/picker/rows.rs): reuse existing row,
  selection, clipping and footer conventions.

### Acceptance

At 48×16, each current registry entry has a readable distinguishing label,
the applied theme is identifiable, and the selected description remains
accessible. At 24×10 the list may scroll and decoration may disappear, but
navigation/accept/cancel still work. Filtering by a canonical ID and accepting
chooses that exact theme; cancel and replay retain their existing semantics.

## 4. Workstream B — file preview allocation

### Direction

Prefer a bounded, automatic allocation policy over new resize bindings or a
configuration subsystem. Give the preview the useful width left over when
result labels are short, while keeping enough list width to navigate real paths.

- The current [shared geometry](../crates/strop-engine/src/editor/prepare/geometry.rs)
  is authoritative: `picker_split` assigns 55% to the Files/Symbols list and
  60% to the Search list. Its nearby comment claims the preview gets the wider
  share; that comment is not evidence of the actual allocation.
- Put the policy in engine-owned view preparation/shared geometry, not a
  renderer-only adjustment. Admission of preview content, cursor coordinates,
  input geometry and every frontend must consume the same decision.
- Base any content-dependent measurement on bounded, already-admitted result
  metadata. Do not scan the entire project, materialize full source lines, or
  allocate a fresh width model for every cell/frame.
- Keep layout stable during typing, streaming and ranking updates. Commit
  sizing at a defined view/layout boundary rather than letting every arriving
  row move the divider. Document that boundary and its width limits.
- Preserve long-path disambiguation, basename/match visibility and the existing
  narrow-screen stacked-preview/no-preview fallbacks. A preview that cannot
  fit should be deliberately omitted, not rendered as a few broken columns.
- Apply compatible geometry to Search/Symbols where they share this code;
  preserve replacement fields, suggestion rows, pinned status rows, selected
  identity and source/error notices.

### Starting points

- [prepare/geometry.rs](../crates/strop-engine/src/editor/prepare/geometry.rs):
  `picker_card`, `picker_inner`, `picker_field_split`, `picker_split`.
- [prepare.rs](../crates/strop-engine/src/editor/prepare.rs): admitted view work.
- [picker/mod.rs](../crates/strop/src/render/picker/mod.rs) and
  [preview.rs](../crates/strop/src/render/picker/preview.rs): consumers of the
  prepared geometry, not independent layout owners.
- [picker/window.rs](../crates/strop/src/render/picker/window.rs): bounded,
  grapheme-safe match windows; this is text clipping, not the card-layout owner.
- [workspace tests](../crates/strop/src/render/picker/workspace_tests.rs).

### Acceptance

The short-filename scenario at 100 columns gives the preview more width than
the list and visibly exposes more source, rather than just changing a ratio
constant without meeting the outcome. Long paths, empty/loading/error results,
streaming updates, repeated resizes and narrow fallbacks remain usable and
stable. Selected resource identity and preview contents never diverge.

## 5. Workstream C — pane identity and boundaries

### Direction

Present paths consistently, make focus clear, and reduce redundant chrome
without removing identity, authority or scroll information.

- Use workspace-relative display paths for local files where unambiguous;
  preserve enough directory context for duplicate basenames and keep outside-
  workspace paths identifiable. Preserve remote host/container identity, dirty
  state and read-only state rather than shortening them into a misleading
  local-looking filename.
- Reuse a common document-aware display projection. The existing modeline
  helper is active-document-oriented: do not call it for every pane and
  accidentally label all panes with the active document.
- Keep pane numbers and the active-pane cue. Use semantic foreground/border
  roles instead of adding multiple saturated accents.
- The apparent double rule has two owners: pane separators and the reserved
  scrollbar column. First reduce visual duplication without changing cell
  ownership. If a column is reclaimed, update preparation, text/caret budgets,
  horizontal scrolling, terminal PTY geometry and any hit testing together.
  Do not merely stop painting a line while leaving inconsistent coordinates.
- Preserve clipped/escaped display safety for tabs, control characters, wide
  and combining graphemes and non-UTF-8 paths. Display shortening must never
  change the underlying filesystem/resource identity.

### Starting points

- [render/buffer.rs](../crates/strop/src/render/buffer.rs): `render_panes`
  constructs titles from raw document paths and paints inter-pane separators.
- [render/statusline.rs](../crates/strop/src/render/statusline.rs): `file_display`
  already separates filename and context and relativizes local paths to cwd.
- [buffer/scrollbar.rs](../crates/strop/src/render/buffer/scrollbar.rs) and
  [prepare/geometry.rs](../crates/strop-engine/src/editor/prepare/geometry.rs):
  scrollbar reservation, text budget and pane geometry.
- [render/clip.rs](../crates/strop/src/render/clip.rs): shared cell-safe clipping.

### Acceptance

Opening the same local file through CLI, picker and `:e` yields consistent
identity presentation. Duplicate filenames remain distinguishable; remote
and local resources cannot be confused. At normal and narrow sizes, focus,
dirty/read-only indicators, cursor placement, source scrolling and terminal
geometry remain correct in both split orientations.

## 6. Workstream D — compact service state, inspectable failures

### Direction

Separate compact service-state presentation from long diagnostic prose.
Use the existing **`:explain` real read-only buffer** for full details rather
than inventing another notification center or a transient popup that loses them.

- Derive the badge from owned service state, not by parsing strings beginning
  with `lsp:`. Retain actionable failure reasons in bounded owner-scoped state
  when needed; unrelated later messages must not erase the only copy.
- Reuse/extend `:explain` to expose the complete relevant failure, server,
  workspace/endpoint and state. Its existing search/motion/close behavior is
  the detail affordance. Make that route discoverable in normal help.
- Start with LSP lifecycle/refusal presentation implicated by the observation.
  Do not expand this into a new universal telemetry/service framework.
- Preserve current transient precedence: operator preview/query errors and
  pending input, then walker/I/O state and messages. Preserve terminal-specific
  escape hints, new-output state and recording/privacy signals. Compacting an
  LSP failure must not swallow a save failure or alter an operation's outcome.
- Keep mode and position anchored under width pressure; degrade optional
  context predictably. A failure indicator must remain distinguishable, with
  full details reachable even when the prose no longer fits the modeline.
- Reject stale/unowned events as today. Failed-to-ready transitions clear the
  obsolete failure state, but successful attachment must not erase independent
  configuration warnings or another workspace's outstanding problem.

### Starting points

- [render/statusline.rs](../crates/strop/src/render/statusline.rs):
  `Modeline::gather`, `fit`, `transient`; already near the repository's file-size
  ceiling. Use the existing `statusline/` module boundary instead of appending
  another large responsibility to this file.
- [lsp/routing.rs](../crates/strop-engine/src/editor/lsp/routing.rs): owned
  Ready/Failed/ServerMessage events currently write formatted `editor.message`.
- [lsp/lifecycle/attach_outcomes.rs](../crates/strop-engine/src/editor/lsp/lifecycle/attach_outcomes.rs):
  attachment and refusal outcomes.
- [picker/status.rs](../crates/strop-engine/src/editor/picker/status.rs): existing
  typed per-project status/reason conventions; do not confuse these pinned
  workspace-symbol rows with a complete global service-status model.
- [editor/explain.rs](../crates/strop-engine/src/editor/explain.rs): existing
  authoritative workspace, worker, document and LSP explanation buffer.
- [statusline tests](../crates/strop/src/render/statusline/tests.rs) and
  [LSP tests](../crates/strop-engine/src/editor/lsp/tests.rs).

### Acceptance

Starting, ready, refused and failed states are visually distinct and scoped to
the correct resource. A long failure does not consume the statusline's entire
budget, and its complete reason remains available through `:explain` after
other input/messages. Recovery and late/stale events do not leave misleading
badges. Save errors, operator previews, terminal hints and privacy indicators
retain their existing precedence and meaning.

## 7. Verification and delivery

Implement each workstream through the existing ownership boundaries. Before
changing exported APIs, inspect references with a language server when
available; no server was configured in the handover-writing environment.

- Use deterministic, isolated behavioral and styled-cell tests for the new
  contracts. Assert meaningful labels, identities, transitions and accessible
  details—not copies of a layout constant, fixed theme counts or source text.
- Preserve the existing suites in `render/picker/workspace_tests.rs`,
  `render/picker/rows/tests.rs`, `render/statusline/tests.rs`,
  `render/buffer/tests/viewport.rs`, `render/theme_tests.rs` and
  `crates/strop/tests/themes.rs`. Update genuinely changed consumer contracts;
  do not re-pin incidental implementation/wording assertions.
- Exercise 100×30, 80×24, 48×16 and 24×10, including repeated resize and both
  split orientations. Cover short/long/duplicate paths, Unicode/tab/control
  display, no results, streaming, warnings and failures.
- Exercise Paper/Paper Dark from the current registry plus existing light and
  dark themes. Check small labels, inactive metadata, status chips, swatches,
  selections, preview overlays and focus cues—not just base text contrast.
  Essential information must not depend on hue alone.
- Run the freshly built TUI/headless path and inspect actual rendered captures;
  tests alone are not visual proof. Use isolated HOME/XDG paths and temporary
  fixtures, not Tarek's preferences or working files. A missing/deliberately
  exiting language server can exercise failure presentation without claiming
  a bug in a real language server.
- A useful reproduction: launch `sample.rs` using the existing
  [Rust fixture](../demos/fixtures/demo.rs), split with `Space w v`, open a
  sibling README using `:e`, return with `Space w h`, filter `Space f` to
  `sample`, inspect `:theme`, then shrink to 48×16. Use the documented
  [headless capture workflow](../docs/terminal-and-scripting.md), including
  `frame`, `--log-file` and `--log-content`. Keep captures source/version-bound
  and remove temporary scaffolding afterward.
- After implementation and smoke, run the required
  `docker compose run --build --rm test` gate from AGENTS.md. Run applicable
  focused tests first if useful; do not claim unrun proof/qualification lanes.
- Update [docs/editor.md](../docs/editor.md), relevant existing help/doc sources,
  [CHANGELOG.md](../CHANGELOG.md), and this plan's status/evidence. Register any
  new discoverable action through the existing command/help machinery.

Delivery is all four completed behaviors plus exercised evidence and explicit
platform limits. No website changes, native theme redesign, version bump,
commit/push or release is requested by this handover-writing task. Native
implementation and any later publication belong to the agent taking it over.
