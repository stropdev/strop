# 0044 — Editable code collections (handoff S5, v1)

Status: v1 + v2 implemented (0.20.0 / 0.21.x). v2 added: multi-region
write-back (real LCS diff, one plan per sync, batched per source),
background loading of unopened sources, and remote sources — write-back
to a permit-less remote buffer refuses naming `:remote edit`; permitted
buffers edit through the gateway and save via `:w`. The differentiating
mechanism is landed and verified: a collection is a real buffer whose
editable excerpts route writes back to their source documents through the
change-plan gateway (0043).

## 1. What a v1 collection is

From a Locations/diagnostics/grep picker, `ctrl-o` opens the listed hits as a
**collection**: one editable buffer of source excerpts under generated headers.

```text
collection: references — 3 excerpts (edits write back on commit; q closes)
── src/main.rs:1-3 ──
fn helper() -> i32 {
    41
}
── src/lib.rs:9-11 ──
…
```

- The collection buffer is a REAL buffer (0001 §4): motions, `/`, operators
  all work. Headers and separators are generated content, not editable spans.
- Excerpt anchors are byte ranges into the source document, remapped through
  the same change-journal position mapping as marks and jumplists
  (`sync_document_positions`), so unrelated source edits keep anchors true.
- A source edit made elsewhere moves anchors; a source edit that *invalidates*
  an excerpt (its recorded content no longer matches) makes the next write-back
  refuse that excerpt by name — never a blind write over newer work.

## 2. Write-back protocol (the heart)

Every commit in the collection buffer syncs (there is no hidden state: the
shadow text is the last-synced canonical rendering):

1. Diff shadow vs current text (contiguous middle change: shared line prefix
   and suffix; v1 deliberately supports one contiguous changed region per
   commit — one user action).
2. The changed region must map fully inside ONE excerpt's view span:
   - Inside one excerpt → the replacement text becomes a change plan
     (`ChangeProducer::CollectionEdit`) against that excerpt's source byte
     span, applied through the revision-checked gateway.
   - Touching a header, a separator, or two excerpts → refused: the view
     regenerates from sources and the message says why. No partial magic.
3. After a successful write-back the view regenerates from the source (the
   source buffer, not the user's text, is authoritative), and the shadow
  resets. Dirty source buffers remain the user's to save — collections never
   save behind the user's back.

## 3. Deliberate v1 boundaries

- Sources are documents already open in the editor; picker hits whose files
  are not open are counted and skipped with a message (async collection build
  over unopened files is the next slice).
- Local documents only; remote sources refuse at build with a named reason
  (remote write authority is a separate admission, 0040).
- One contiguous changed region per commit; overlapping excerpts of one source
  are merged at build so an edit never applies twice.
- Source closes invalidate the collection at next sync (named message).
- No persistence of collection definitions yet (saved collections are S7).

## 4. Tests

Two excerpts of one document (merged overlap), header edit refused, cross-
excerpt edit refused, single-excerpt edit lands on the source through the
gateway with a receipt, source edited elsewhere remaps anchors, source
content divergence refuses write-back, source closed refuses, regeneration
matches source after write-back, `q` closes.
