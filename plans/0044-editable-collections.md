# 0044 — Editable source collections

Status: implemented in 0.20/0.21 and extended by 0051 for 0.29.0. The live
journal-based protocol below supersedes the original commit-only/LCS design.
A collection is a real buffer; its editable bodies refer to real source documents.

## 1. Source and view identity

Ctrl-O from locations, diagnostics or grep opens the listed source hits. Unopened
local sources load on owned background requests. Remote hits retain their endpoint
identity and use the existing remote document/write-permit contract (0040).
Late loads can finish the requested collection but cannot steal a newer focus.
Unavailable sources are counted and named; a partial build is not silent success.

Each excerpt retains a source DocumentId, source byte span, view byte span, source
line range and independent match spans. Nearby context merges; match counts do not
collapse into excerpt counts. The default context is two lines; `+`/`-` adjusts it.
Cards, source numbers and omitted-line gaps are typed projection rows, not strings
parsed back into write authority. Names, syntax and indentation belong to the source.

## 2. Live publication

1. Collection mutation journals retain each sequential user operation. Even a
   net-zero change that touched generated chrome is not an authorized source edit.
2. Every edit must project into one editable excerpt body. Headers, gaps and edits
   crossing excerpt boundaries refuse visibly and refresh from authoritative sources.
3. The journal derives source replacements. Preflight every affected source through
   the shared core prepared-replacement boundary before publishing any source change.
   Readonly, closed, stale and unauthorized remote sources refuse without partial edits.
4. Publish through source mutation leases. Keep the ordinary Insert undo group open,
   while source journals immediately remap and update every dependent collection view.
   Ordinary typing splices affected excerpts rather than flattening/rebuilding the view.
5. Synthetic final-newline separators remain projection data, never file bytes.
   Source buffers stay dirty until an explicit, confirmed save.

Source changes outside the collection use the same journal invalidation path.
Syntax requests are aggregated by visible source window; a collection is never
parsed as if unrelated files were one language unit. Delimiter endpoints project
only through excerpts of the same source. Unknown/offscreen results do not scroll.

## 3. History, persistence and navigation

Collection `u`/Ctrl-R use retained source-group receipts and exact history positions.
Preflight source liveness, write authority, history position and revision capacity
before moving a group. A refusal retains the receipt so resolving the blocker and
retrying remains possible. Intervening independent source edits are not undone blindly.

`:w` saves dirty source documents, not the projection. An explicit output path is
refused. Only successfully admitted source writes enter the collection's pending set;
an unrelated or duplicate completion cannot count toward it. `:wq` closes the active
view only when all admitted saves confirm current source revisions. Refusal, failure,
cancellation, remote uncertainty or newer edits keeps the view and unsaved text.
`:q` closes the view without discarding source buffers or implicitly saving them.

`g<Space>` opens the full source at the projected position; Enter on a file header
opens its source. Ctrl-O/Ctrl-I use the shared full navigation record, preserving
caret/selection, byte-anchored viewport and horizontal origin across edits and resize.

## 4. Boundaries and verification

Collection definitions are not persisted as named investigations. Visibility is
not a write grant: remote permits, partial snapshots and container readonly policy
remain authoritative. No local-path fallback is introduced for another namespace.

Behavioral coverage lives in `editor/collections/tests/{mod,live,ownership}.rs`,
remote save receipt tests, shared transaction conformance and renderer cell-grid
regressions. The 0051 acceptance ledger records real source-colored collections,
live split editing, context/source round trips, grouped history and save evidence.
