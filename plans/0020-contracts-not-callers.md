# 0020 — Contracts, not callers (0.9.1 + 1.0 candidates)

Status: accepted (2026-09-05 third review round — 15 findings, all
verified against source at 2b6d2e7). The reviews agree with each other
and with our roadmap: no new kingdoms; finish the roads. This plan is
the pothole list and the two structural follow-ups.

## P0 — the 0.9.1 set (each with a regression test)

1. **`:w path` overwrite policy** — save_as adopts the destination and
   clears the disk baseline BEFORE writing, so ordinary `:w b.txt`
   overwrites an existing file silently, and a failed write leaves the
   buffer married to the attempted path. Fix: refuse existing targets
   without `!`; adopt the path only after successful persistence;
   a failed write changes nothing.
2. **Streaming pickers die on the second keystroke (introduced in
   0.9.0)** — query changes respawn workers with a fresh channel that
   never forwards to AppEvent; the TUI waits on AppEvent only. Fix:
   forward respawns through the retained app channel; tag messages
   with (picker id, query generation) and drop stale ones — this also
   kills the old-stream-into-new-picker leak the review noted.
3. **Project replace mixes byte offsets with Ropey char-indexed
   slice** — panics or mis-verifies with multibyte text before a match.
   Fix: byte-exact verification via Buffer's byte API; direct rope
   slicing stays behind the document interface.
4. **`d/foo<Enter>` is broken in normal mode (introduced in 0016)** —
   the walker appends the token "enter" to the motion string; the
   grammar's search parser waits for `\r`. Fix: op-pending search text
   completes on Enter as `\r`. Preview: the composition IS the window
   (search text), restore incsearch-preview through the typed plan.
5. **Syntax cache under-invalidation** — key is len+first+last byte;
   same-length middle edits keep stale colors. Fix: revision-keyed
   invalidation now; incremental tree-sitter is the roadmap item.
6. **LSP freshness compares unrelated counters** — my 0.9.0 guard
   compared the server's protocol version against the buffer's edit
   epoch (different clocks; I shipped it knowingly). Fix: map each sent
   LSP version → buffer revision at send; accept diagnostics by that
   map. Encoding: look it up per event (the forwarder captured it
   possibly pre-init).
7. **Git hunk discard skips the undo transaction** — mutations without
   begin/commit, and the restore path uses str::lines + LF joins
   (CRLF/final-newline loss). Fix: same transaction as typing; the
   hunk's own byte-precise lines supply the restore text.
8. **Unopened-file replace has second-class persistence** — no undo
   coverage, no permission preservation, misleading "u per buffer"
   message, a third copy of the temp-write logic. Fix: reuse the
   Buffer::save machinery, preserve mode, honest message.
9. **Grep rows get fuzzy-filtered by the regex query** — rg's matches
   for "foo|bar" contain no literal "|", so the fuzzy pass hides them;
   the replace apply-set can include rows the user never saw. Fix: the
   source's rows are the apply set; fuzzy filtering applies to local
   narrowing only, never to what was matched upstream.
10. **UTF-8 paste/search off-by-one** — charwise paste inserts at
    cursor+1 byte (mid-char before a multibyte char → inserts before
    it); repeat-search slices from a possibly-mid-char byte (panics).
    Fix: boundary-aware stepping everywhere a caller adds 1.
11. **Failed first open strands the pane** — the scratch doc is dropped
    before the read succeeds. Fix: fallible I/O first, swap on success.
12. **Resize is swallowed** — the reader thread drops it; an idle
    terminal never redraws on resize. Fix: AppEvent::Resize.
13. **Sessions only exist for pathless launches** — `strop file.rs`
    gets no persistence. Fix: initialize state_dir always; restore
    selection only in dir mode.

## P1 — structural (same plan, lands right after)

14. **Universal anchor mapping** — edits adjust the active pane's
    cursors; marks, jumplists, and other panes' selections hold raw
    offsets that go stale. One transaction emits a ChangeMap applied to
    every anchor in the workspace, with an explicit insertion bias.
15. **Project-config trust gate** — a cloned repo's languages.toml
    overrides server `command`/`args` and auto-executes on attach. Fix:
    command overrides from a project layer require a one-time trust
    confirmation per project (persisted in the state dir); init-options
    (pythonPath-style) stay free — that's the daily-driver case.

## Pushbacks (recorded, not adopted)

- **"Prototype a Neovim-based version first"** — rejected. The
  product IS the independent engine; the preview promise has no
  meaning as a nvim plugin.
- **nucleo vs the custom scorer** — deferred to an evaluation with
  numbers, not a swap on principle. The picker scorer is 60 lines and
  not a measured pain point.
- **git CLI over git2** — rejected (0001: hot paths stay in-process).
- **Tokio at the core** — rejected by both reviewers and us; services
  keep it behind their boundary.

## Roadmap additions (ordering)

- Incremental tree-sitter (revision-tagged edits into the old tree) —
  1.0 item, pairs with the perf bench suite.
- `DocumentSource` enum (File/Scratch/GitRevision/Output/Help) —
  formalizes surfaces-as-buffers; 1.0.
- PathBuf document paths (non-UTF-8 filenames) — 1.0.
- Perf benchmark suite (1/10/100MB, 100k lines, 1k cursors, tail
  latency) — precedes any perf claims; 1.0 gate item.
- Capability-shaped plugin boundary (expected-revision edits) —
  unchanged: after the command/event vocabulary settles.
- strop-picker joins the workspace members (hygiene, now).
