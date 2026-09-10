# 0048 — Preserve Escape before the next key

Status: **implemented in 0.23.0**, 2026-09-10. Investigated source
baseline: `fc3fbfa673171e4260530f45f0b170dac2d25f68` (0.22.1). §A and §B
landed as proposed: the terminal adapter expands Alt-modified keys to
Escape + base key in FIFO order (with the Alt+Ctrl+C and unmapped-base
edges pinned by unit tests), requests
`DISAMBIGUATE_ESCAPE_CODES` with idempotent pop on exit, and keeps the
CSI-u Control aliases (Ctrl-[/I/M) mapped to their legacy meanings. The
§2 PTY matrix passes on the changed binary: coalesced `ESC hkl` saves
`writewrite` with the letters as Normal-mode motions; separated and
CSI-u forms unchanged; arrows, bracketed paste and clean restoration
verified on a real PTY. Legacy terminals without the protocol keep the
§A fallback; `ESC [`/`ESC O` collisions remain irreducible there (§C).

## 1. The reported failure

After typing in Insert mode, pressing Escape and immediately typing motions
sometimes inserts those letters instead of leaving Insert mode. The user also
observes this with `Esc+k`, not just `Esc+h`, and reports that other following
keys behave similarly.

**Confirmed cause: Escape can be lost at the terminal-to-editor boundary.**
This is not merely a late repaint: in the reproduced failure the editor never
receives an Escape event and the unwanted letters are saved into the file.
The decoder rule is general, not an `h/j/k/l` keybinding problem.

## 2. Reproduction and evidence

Ran the real TUI through Linux PTYs, with an 80×24 terminal, an empty local
`text.txt`, private HOME/XDG directories, and full-content traces of synthetic
fixture text only. Typed `iwritewrite`, observed the resulting Insert state,
then injected the following bytes. Separately acknowledged Escape before
`:wq` to save the actual resulting buffer and exit cleanly.

Both binaries produced the same results:

- A frozen copy of the installed `strop 0.22.1` binary.
- `/app/target/debug/strop` extracted from cached Docker image `8e925f90268c`.
  The image's `terminal.rs`, `editor/insert.rs`, and `Cargo.lock` SHA-256 hashes
  matched the investigated source snapshot.

| Input after `writewrite` | Editor keys received | State before cleanup | Saved text |
| --- | --- | --- | --- |
| One PTY write: `b'\x1bhkl'` | `h`, `k`, `l`; **no Esc** | INSERT, cursor byte 13 | `writewritehkl` |
| `b'\x1b'`, observe Normal, then `b'hkl'` | Esc, `h`, `k`, `l` | NORMAL, cursor byte 9 | `writewrite` |
| One PTY write: `b'\x1b[27uhkl'` | Esc, `h`, `k`, `l` | NORMAL, cursor byte 9 | `writewrite` |
| Ordinary typing control: `b'hkl'` | `h`, `k`, `l` | INSERT, cursor byte 13 | `writewritehkl` |

All eight completed editor cases exited successfully through `:wq`. The
separated-input comparison waits for an observable state, not an arbitrary
sleep. One preliminary probe had an incorrect expected cursor byte (1 rather
than 9); its timeout was a probe error, corrected before completing the matrix.

A separate throwaway binary using **Crossterm 0.28.1** and its real
`event::read()` showed the intermediate events directly:

```text
bytes: ESC h k l
KeyEvent { code: Char('h'), modifiers: ALT, kind: Press, ... }
KeyEvent { code: Char('k'), modifiers: NONE, kind: Press, ... }
KeyEvent { code: Char('l'), modifiers: NONE, kind: Press, ... }
```

Both separated Escape and CSI-u Escape (`ESC [ 27 u`) decoded as an explicit
`KeyCode::Esc` followed by the three ordinary characters.

A real PTY fidelity comparison with `nvim 0.9.5 -u NONE -i NONE -n` received
`b'iwritewrite\x1bhkl:wq\r'` in one write and saved `writewrite\n`, exit 0.
The final newline is Neovim's normal file-write behavior; no motion letters
were inserted.

### Evidence retained outside the repository

Everything below is under `/tmp/strop-escape-probes/`, not a new project test
stack:

- `probe.py`: real PTY driver; ordinary invocation reruns the matrix in fresh
  fixture directories. `--resume` was used to finish the preliminary run.
- `decoder/`: throwaway Crossterm-only event reader, pinned to `=0.28.1`.
- `results.json`: the seven completed cases from the resumed run.
- `installed-legacy-merged/trace.jsonl` and `text.txt`: the first completed
  installed-binary failure, preceding that resumed run.
- `baseline-legacy-merged-144957498124046/trace.jsonl` and `text.txt`:
  cached-build failure; the other comparison trace paths are in `results.json`.
- Each fixture directory also retains `terminal.bin`, the real terminal output.
- `oracle.py` and `nvim-oracle-145050331291841/`: the Neovim comparison.

The table and byte sequence above are the durable reproduction contract;
implementation must not depend on these temporary paths surviving.

## 3. Root cause and ownership

### Crossterm combines the legacy bytes into an Alt event

In Crossterm 0.28.1:

- `src/event/source/unix/mio.rs:94–119` reads a batch of terminal bytes.
- Its `Parser::advance` at lines 198–219 passes whether more bytes remain in
  that batch into `parse_event`.
- `src/event/sys/unix/parse.rs:35–88` holds an initial Escape while more input
  is available. For an ordinary following character, it parses that character
  and adds `KeyModifiers::ALT` instead of emitting a separate Escape.

That is legacy terminal ambiguity: an actual Alt chord and Escape followed by
a character can have exactly the same bytes. Read batching therefore matters.
A lone Escape at the end of a non-full read is emitted without a fixed
Escape-specific wait in this path. Do not invent a measured millisecond
threshold from these experiments.

### Strop then discards the only surviving Escape-prefix information

In `crates/strop/src/terminal.rs`:

- Lines 36–70 read Crossterm events and enqueue at most one application event
  per key event.
- `key_from_event`, lines 195–232, handles selected Control combinations, then
  falls through to `KeyCode::Char(c) => Key::Char(c)` at line 230.
- No branch preserves or interprets `KeyModifiers::ALT`.
- Startup, lines 28–32, enables raw mode, alternate screen and bracketed paste,
  but does **not** request keyboard-protocol Escape disambiguation.

Thus `ESC h` becomes `Alt+h`, then plain `h`. The mode machine cannot exit
Insert because it never sees `Key::Esc`. Other ordinary characters go through
the same branch. This is not a reason to special-case particular letters.

Downstream code is not the source of this reproduction:

- `crates/strop-engine/src/editor/events.rs:109–112` forwards terminal keys.
- `editor/insert.rs:123–181` changes mode synchronously when Escape arrives.
- `editor/events/channel.rs:13–16` sends FIFO and unparks the driver. The
  terminal loop's 16 ms idle park is wakeable, not an Escape timeout.
- Deferred editor input is also queued in order. Making Escape jump ahead of
  earlier edits is not the remedy.
- Existing semantic-key/headless tests bypass the raw-byte/Alt conversion.
  Plan 0006 already names `ESC merging into Alt+<key>` as a terminal-boundary
  risk; coverage must exercise that boundary rather than feed `Key::Esc` only.

**Limits of the finding:** the user's physical Windows Terminal version,
transport buffering, and natural inter-key timings were not captured. The PTY
experiment demonstrates a concrete coalesced-input mechanism matching the
report; it is not a general input/render latency benchmark and does not rule
out separate performance problems.

## 4. Proposed implementation: one terminal-boundary cutover

Keep the fix in the binary's terminal adapter. Do not change the pure grammar,
preview resolver, Insert handler, or typed input-owner priorities.

### A. Preserve the Escape prefix during key normalization

Replace the one-key-only conversion boundary with a small allocation-free
zero/one/two-event result, consumed by the existing input sender. Reuse existing
key and Control-key conversion rules rather than building a second keymap.

- Filter `KeyEventKind::Release` **before** any expansion. Press and Repeat
  retain their existing handling.
- For a supported Alt-modified key, remove only the Alt bit, normalize the
  remaining key/modifiers, and deliver **Escape, then the base key/action**.
  Both use the existing priority input lane, in order and exactly once.
- Apply the policy generally, not just to motion letters or only in Insert.
  Prompt fields, pickers, overlays, macro recording and tracing must consume
  the same normalized semantic input through existing dispatch.
- Preserve remaining modifier information during conversion. Do not reinterpret
  Ctrl/Shift combinations by blindly taking only the `Char` payload.
- Keep Ctrl-C's quit-intent policy at this boundary. Its current early branch
  precedes key conversion; an expanded Alt+Ctrl-C must not accidentally bypass
  the leading Escape or duplicate the quit action.
- Avoid a per-event `Vec`, sleeps, new timers, process spawns, or any `await`.
  A bounded stack result is enough. The normalized keys already flow through
  existing input traces; no new tracing project is needed.

**Explicit Alt policy:** Strop's `editor::Key` has no independent Alt binding
representation. Treat Alt as an Escape prefix consistently at this boundary,
including explicit Alt events from an enhanced terminal. A physical Alt chord
can therefore leave Insert mode and run the base command. This is intentional,
not a claim to distinguish identical legacy byte streams. If independent Alt
bindings are introduced later, they require a deliberate key-model contract;
do not silently strip Alt now or create a mode-specific shortcut workaround.

### B. Request unambiguous Escape encoding from capable terminals

Use the existing Crossterm APIs; the reproduced CSI-u path already works with
0.28.1, so a dependency upgrade alone is not the fix.

- After entering the alternate screen, request
  `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES)` (`CSI > 1 u`).
  Request only the needed enhancement, not all-key or release reporting.
- Pop with `PopKeyboardEnhancementFlags` **before** leaving that screen.
- Extend terminal restoration for success, error, and panic. The current panic
  hook and `TerminalLease::drop` restore separately: make restoration
  idempotent so a panic cannot pop twice, especially after returning to the
  main screen's separate protocol stack.
- Unsupported terminals must still start and use the legacy policy in A.
  Do not require terminal configuration or introduce a TERM-name allowlist.
  Account for Crossterm's native-Windows `Unsupported` implementation; WSL
  uses the Unix path exercised here. Real terminal I/O errors still surface.
- Avoid introducing a capability-query stall or a second concurrent
  `event::read` owner. The protocol quickstart permits push/pop without a
  probe. If detection is needed, keep it at startup and bound it, not on input.

Disambiguation also changes how **Control chords** are encoded. Before
requesting it, preserve existing semantic aliases across raw-byte and CSI-u
representations, particularly Ctrl-[ → Escape, Ctrl-I → Tab, Ctrl-M → Enter,
and the already supported Control actions/quit intent. Those encoded chords
must not fall through to literal `[`, `i`, or `m`. Ordinary Enter/Tab/Backspace
remain supported as before.

The protocol spec currently lists Microsoft Terminal among implementations,
but this investigation did not establish the user's installed version or
whether an intermediary forwards the protocol. Legacy fallback is required.

### C. Respect irreducible legacy ambiguity

Do not split raw input at every `0x1b`: that breaks arrows, function keys,
CSI/SS3 sequences and bracketed paste. Let Crossterm decode real terminal
sequences.

A covers keys that Crossterm exposes as Alt events. Legacy `ESC [` and `ESC O`
can instead start CSI/SS3 sequences; the original intent is not recoverable
universally after parsing. Do not promise that Alt expansion solves every
possible Escape/key collision on an unenhanced terminal. B removes that
ambiguity where supported. Document this boundary instead of inventing a
lossy parser heuristic or an input-dropping workaround.

## 5. Acceptance and verification for the implementing agent

1. **Keep a focused consumer-level regression at the terminal adapter.** Start
   a real `Editor` in Insert with `writewrite`, pass an actual
   `KeyEvent(Char('h'), ALT)` through the production normalizer and event
   dispatch, and assert unchanged text, Normal mode, and cursor byte 8. This
   catches both lost Escape and a fix that drops the following key. Exercise
   a non-motion command as well to prevent an `hjkl`-specific patch.
2. **Cover distinct boundaries, not a padded alphabet table.** Preserve shifted
   and non-ASCII character identity; release events must do nothing, repeats
   must retain their intended action, and supported Control combinations must
   retain their semantics. Include raw versus CSI-u Control aliases affected
   by B. Assert buffer/mode/cursor or the actual action, not helper field copies.
3. **Protect modal consumers and ordering.** A pending editable line/picker
   must observe its existing Escape transition before the following key.
   Ordinary text bursts must remain intact; Escape cannot overtake earlier
   edits. Bracketed paste is one text event and must not be normalized as keys.
4. **Run a real PTY smoke on the changed binary.** Repeat the legacy-coalesced,
   separated and CSI-u cases in §2. Assert saved file contents, semantic input
   order and clean exit. Synchronize startup on observable readiness; write
   the ambiguous bytes together. Do not use sleeps to make the bug disappear.
   Verify arrows, paste and normal terminal restoration on the real surface.
5. **Verify protocol lifecycle and compatibility.** Use a capable terminal or
   a small protocol-aware PTY peer that sends CSI-u only after the request;
   exercise an unsupported/no-response terminal too. Check that normal exit
   and error/panic restoration restore the prior keyboard mode once. The
   investigation's manually injected CSI-u control proves decoding only, not
   negotiation or lifecycle correctness.
6. **Run the repository gate after implementation:**
   `docker compose run --build --rm test` (fmt, locked workspace Clippy with
   warnings denied, and locked tests). Retain regression coverage in Rust
   following 0006; do not import the throwaway Python probe as a permanent
   test framework. Update this plan and the existing changelog with the actual
   completed behavior and any legacy limitations.

This investigation ran the binaries and decoder/oracle experiments, not the
project-wide gate: no shipping code was changed and no fix is claimed landed.

## References

- [0006 — E2E harness](0006-e2e-harness.md), especially byte ambiguity and the
  terminal-boundary tier.
- [0008 — Input layer as data](0008-input-layer-keymap.md): one semantic
  dispatch model, not a second terminal-specific keymap.
- [Kitty keyboard protocol: quickstart and disambiguation](https://sw.kovidgoyal.net/kitty/keyboard-protocol/).
- [Microsoft Terminal's keyboard-protocol implementation](https://github.com/microsoft/terminal/pull/19817).
- [Crossterm 0.28.1 decoder](https://github.com/crossterm-rs/crossterm/blob/0.28.1/src/event/sys/unix/parse.rs)
  and [Unix input source](https://github.com/crossterm-rs/crossterm/blob/0.28.1/src/event/source/unix/mio.rs).
