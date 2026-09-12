# 0054 — One filesystem workspace: browse, edit names, review operations

Status: requested architectural/product handoff, not implemented. This is a
separate substantial filesystem milestone alongside the search refinement in
[0053](0053-unified-search-workspace.md), not an excuse to omit any of 0051's
current-release requirements. GUI and embedded-terminal implementation are not
prerequisites.

## 1. Product decision

**One real Directory buffer for local files and SSH, with capability-aware file
operations and the same navigation model.** Adapt the existing container listing
into that model without granting container write authority.

The user should be able to locate, inspect, open, create, rename, move and organize
files without dropping into an external file manager or terminal. `:e` should not
feel like an unrelated, weaker entry path. Browsing is not permission to mutate;
a read-only directory presentation is not proof that the backing filesystem is
incapable of an explicitly requested operation.

Borrow three concrete ideas:

- **Dired:** an ordinary navigable directory buffer; marked entries; explicit
  operations; local and remote locations; open buffers follow confirmed renames.
- **Oil/Wdired:** edit filenames with normal editor grammar, then commit a proposed
  set of filesystem changes. Do not execute a delete because one row disappeared.
- **Yazi:** a legible current-directory view, useful preview, clear parent/child
  travel, selection and visible background operations. Do not copy keybindings
  that steal Vim motions, marks or Strop's Space leader.

The original [0001 §3](0001-vision-and-design.md) already promises a Dired/Oil
lineage: rename by editing a line, delete with `dd`, copy with yank/paste and
confirmation. **That remains required. A read-only tree plus a New File dialog
is not completion of this milestone.** Explicit single-operation dialogs land on
the same planner as modal filename editing, not a competing implementation.

### Required delivery ledger

| ID | Required result |
| --- | --- |
| F01 | Shared namespace-aware Directory model for local, SSH and existing read-only container listings; no parallel RemoteDirectory implementation left behind. |
| F02 | Coherent CLI / `:e` / `:browse` / current-file reveal, path completion, parent/child navigation and view restoration. |
| F03 | Readable names, type/source identity, line/row information where meaningful, metadata, backgrounds, preview, filtering and large-directory behavior. |
| F04 | Explicit exclusive file creation and directory creation locally and on capable SSH hosts, including reviewed missing-parent creation. |
| F05 | No-clobber rename and same-filesystem move of supported files/directories, including dirty/open-buffer and open-descendant handling. |
| F06 | Regular-file copy, local Trash where supported, and separately explicit permanent removal of files/empty directories; safe supported SSH equivalents, never container writes. |
| F07 | Real modal filename-editing drafts: rename/create/remove/copy using text operations and provenance; review before filesystem mutation. |
| F08 | One checked filesystem plan, common review/receipt surface, owned operation ledger, cancellation/partial/unconfirmed reconciliation and guarded recovery actions. |
| F09 | Directory-to-search scope bridge; local search and read-only search on capable SSH hosts use the same 0053 workspace, never a local-path fallback. |
| F10 | Resource relocation propagated to document bindings, saves/permits, LSP, collections, history, previews and directory views without losing text or stable document IDs. |
| F11 | Small capability/identity boundaries, bounded asynchronous workers, replay safety, frontend-neutral models and readable modules. |
| F12 | Deterministic contract coverage, actual local/SSH/TUI walkthroughs, container refusal checks, documentation and the integrated release gate. |

All F requirements are load-bearing for the filesystem milestone. Phases below
are dependencies, not permission to call an early browse-only phase complete.
The implementation owner records evidence and limitations per F ID. An F item
can be removed only by explicit user approval recorded in 0028.

“Capable SSH” is not permission to return Unsupported for every host. The
supported POSIX/Python/OpenSSH fixture must exercise real successful creation,
rename/move, copy and removal. SFTP-only, permission-limited or unsuitable hosts
remain honestly restricted. Capability refusal is a tested product behavior,
not an implementation placeholder.

## 2. Current evidence and what can actually be reused

### Runtime baseline

A frozen **strop 0.28.0** executable was run against a private fixture through the
real headless input/event/render path. Its SHA-256 is
`db347d0e49b6754970dc6b5d6661101d53b07e53e0d44ec5695df741c5c71398`.
The working source already contained another session's 0051 edits; no build or
formatter was run against that in-flight tree.

Observed walkthrough:

```text
keys :e sub<cr>
settle
state
frame
keys :e sub/new-note.txt<cr>
settle
state
keys icreated through the editor<esc>:w<cr>
settle
state
frame
```

- `:e sub` reported `Is a directory (os error 21)` and retained `a.txt`.
- `:e sub/new-note.txt` opened an empty named buffer. After typing and `:w`, the
  file contained exactly `created through the editor`, and the buffer was clean.

Thus local creation is already possible through ordinary editing and saving;
what is missing is a coherent filesystem workflow and explicit operation model.
Do not describe creation as universally impossible, and do not break this Vim
new-file workflow while adding a file manager.

### Source route and primitive inventory

| Area | Current behavior / owner |
| --- | --- |
| Local file open | `Buffer::open`: file read; NotFound creates an empty named buffer; directory read fails. |
| CLI local directory | `main.rs` selects the initial cwd and starts the flat Files picker, not a Directory buffer. This differs from `:e directory`. |
| `:browse` | Local Browse intent is rejected by `editor/io.rs`; remote listing is supported. |
| Remote directory | `DocumentSource::RemoteDirectory`, `document/remote.rs`, `remote/directory.rs`: typed entries, parent row, Enter/Backspace/`-`, filtering and child reveal on returning upward. |
| Path completion | `remote_completion/` completes remote destinations/paths. Local Ex operands do not have the equivalent path-completion path. |
| Local writes | `strop-core/src/buffer/io.rs`: prepared owned saves, private temp file, atomic publication, conflict checks and no-clobber publication for explicit new-name saves. This is not yet a general file-operation API. |
| SFTP | `strop-remote/src/transport/wire.rs` explicitly implements a read-only v3 contract: open/read/stat/list, bounded packets and native filename bytes. No public create/rename/copy/remove operations. |
| Remote mutations | [0040](0040-remote-editing-and-saving.md)'s fixed Python helper owns protected saving of an existing regular file. Its private stage mkdir/replace/cleanup are not general filesystem operations. |
| Containers | Listing/read exist; mutations are intentionally refused by capability policy. |
| File rename | No file-rename command or general relocation owner. Existing `:rename` is **LSP symbol rename** and must keep that meaning. |

The reusable foundations are 0042's `Filesystem`/`ResourceLocation`, the remote
directory's structured entry map, existing owned-worker/tape admission, protected
save safety patterns and the common change review/receipt presentation. They are
not a finished generic filesystem provider or a safe copy/rename library.

## 3. Entry paths and navigation

### Explicit entry contract

| Entry | Behavior |
| --- | --- |
| `strop DIRECTORY` | Start in that directory's shared Directory buffer. Initial workspace/cwd selection may happen at startup as today; subsequent browsing does not mutate process cwd. |
| `:e FILE` | Open the resolved file normally. Preserve dirty-buffer/force and source-authority rules. |
| `:e DIRECTORY` | Resolve its kind on an owned worker and open the shared Directory buffer. No attempt to read a directory as file contents. |
| `:e missing-file` | Preserve the local empty-named-buffer-until-`:w` behavior. Do not create missing parents or truncate another file. Remote new-file creation is explicit through F04; no new remote save-as permission is implied. |
| Bare `:e` / `:e!` | Remain reload/edit commands, not a hidden file-manager shortcut. Preserve the existing remote refresh contract; any local Vim reload correction is a separately tested grammar fix, not accidental browser dispatch. |
| `:browse [LOCATION]` | Open a directory in any supported namespace. Without an operand: active directory, else active source's parent, else the current workspace root. A supplied non-directory gets an honest diagnostic. |
| `Space e` | Reveal the current source in its parent Directory buffer; keep the selected child visible. In a Directory buffer, reveal its current location without rebuilding a fresh unrelated view. |
| `Space f` | Keep the quick recursive file finder. It is not the directory browser. |

`Space e` was unassigned in the inspected keymap; register it once in the common
command/help listing. Sources in collections use the active excerpt's real
location. A scratch/generated buffer without a source falls back to the explicit
workspace root, not a guessed path made from its title.

Resolve local, SSH and container targets at the existing textual location boundary.
After resolution, pass typed locations. A filename obtained from a directory
listing that happens to contain `ssh:` is still a filename in that namespace.
Do not feed it back through a URI parser and do not use display strings as keys.

### Inside a Directory buffer

- Normal Vim motions, search, marks, selections and yanks work on the real buffer.
  Do not steal `m`, `f`, `t`, `/`, `?` or `Space` to imitate another file manager.
- Enter opens the selected structured entry; a directory enters it, a file opens
  its source. `-`, Backspace and the explicit parent entry go up, directory-locally.
- On returning upward, reveal the child just left. On returning to a previously
  visited directory, restore its filter, selected entry, view top and horizontal
  position. Key by location/entry identity, not the old row number.
- `q` restores the originating view using the existing generated-buffer return
  contract; pending filename drafts require an explicit keep/discard decision.
  Returning from an opened file must retain a usable route back to the directory.
- Refresh re-lists through an owned request. A failed refresh keeps the prior
  listing visibly stale; it does not replace it with a fake empty directory.
- Existing split/window actions open the selected entry through the same source
  opener. Do not add another set of direct file reads for preview or split-open.

Directory navigation does not change global cwd, another pane's namespace, LSP
roots, pending jobs or the destination of an already prepared operation.

### Path input and completion

Use one path-completion session contract for local and SSH Ex operands and file
operation destination fields. Reuse the existing field component and remote
completion ownership/cache model; extend with a local `read_dir` worker instead
of implementing a second input editor.

Completion lists the captured parent namespace, distinguishes files/directories,
shows native labels safely, preserves spaces and returns a typed selected target.
No blocking filesystem calls in Tab handling. Replies own field revision, location,
connection incarnation and request ID; stale replies cannot insert a path into a
new command. Do not enumerate an entire tree to complete one directory component.

File operands and destination names are **not the 0051 search-query language**.
`language:rust`, leading `-`, quotes and spaces can be literal filenames. Use the
existing location/argument codec at textual boundaries and retain native paths
inside the model. A displayed directory row is never reparsed as an Ex command.

## 4. Directory presentation and scope

A Directory buffer occupies its ordinary editor pane; it is not necessarily a
permanent sidebar and does not replace the large Search card. Use a current-folder
list plus an optional preview at useful widths. At narrow widths, keep the list
and collapse/stack preview. A mandatory three-column tree would waste the small
pane case; breadcrumbs supply parent context without requiring another tree.

### What the user sees

- A strong current directory name and quiet full breadcrumb; namespace/host or
  container identity always visible when ambiguity is possible.
- Clearly distinguish directories, regular files, symlinks, inaccessible entries
  and unknown type. Directory grouping, stable name ordering and a useful selected
  row matter more than decorative icons; no Nerd Font dependency.
- Strong filename, subdued metadata, restrained directory/type accents, a
  full-width selection band, and distinct base/header/preview background roles
  shared with 0050/0051/0053. No rainbow per-file backgrounds or nested boxes.
- Size/permissions and other actually available metadata. Unknown is not zero,
  inaccessible is not empty, and loading is not a guessed stat result.
- Preview source header and source line numbers for text. The directory listing's
  own row number is not a source-file line number. Binary/unavailable previews
  explain their state and never execute the selected file.
- Marks, staged changes, operation progress and refusals are visible without
  confusing them with text-buffer dirty state or Git changes.

### Visibility and filtering

Browsing a directory and recursively searching a project are different scopes.
A literal directory view should show its children, including dotfiles and ignored
entries by default, with optional filtering and subdued ignore metadata. It must
not hide an existing `target/` or `.git/` merely because recursive project search
normally excludes those trees. Explicit browsing is not an instruction to search
or modify their contents.

Keep the user's hidden-file preference and expose the effective setting. Reuse
0051's query field/parser and file-selection predicates for a **current-folder
filter**, not another ad hoc `name.contains` parser. Label its scope. Parent/up
navigation is a separate control, not a fake matching file. Unsupported predicates
or unavailable ignore information must be diagnosed or unavailable by capability,
not silently ignored. Do not read every file body to filter filenames.

The same qualifier has the same meaning wherever supported; contextual defaults
are visible. In particular, a Directory view showing ignored entries must not
silently broaden a replacement search's ignore policy when invoking Search here.
All directory filter/history state is per captured directory view.

### Search here

Search here opens **0053's workspace** with an immutable namespace/root scope;
it does not change cwd or open a different grep widget. Local execution reuses the
existing provider. For supported SSH hosts, add a bounded owned read-only `rg`
provider through the existing fixed execution/argv transport, with `--no-config`,
argument separation and native filename decoding. Never interpolate paths or the
query into a shell command. Coalesce/retire outstanding remote work; do not leave
one SSH/rg process behind for every keystroke.

SFTP-only hosts without the required search execution capability still browse
normally and receive a specific Search-here refusal. No implicit whole-tree
copy/download to make remote search appear implemented. Remote result acceptance,
preview and collection promotion carry remote resource identity end to end.

This adds remote **read-only search**, not blanket project-replacement authority.
Where project replacement is unsupported, With/Review capability is explicitly
unavailable; regular-file write permits are not automatically granted across the
match set. Future remote project replacement needs its own bulk authority design.

## 5. Actions, marking and capability matrix

Use **`Space a` as the Directory-local actions entry**, retaining its existing
code-action role in source buffers. Context selects one typed action owner; avoid
a fragile branch-order chain. The action palette and Ex commands invoke the same
operation preparation API. Keep `:rename` for symbols and reserve `:file` for Vim's
buffer-name semantics; use the **`:fs` namespace**, including `:fs rename`, for
filesystem operations.

Required action vocabulary: New file, New directory, Rename, Move, Copy, Trash,
Delete permanently, Edit names, Toggle mark, Clear marks, Search here, Refresh,
Copy path, Operations/history and Verify outcome. Provide the corresponding
`:fs …` commands; one canonical command per action, not parallel aliases.
Creation/rename/move dialogs have dedicated name/destination fields with path
completion, explicit source context and a preview of the resolved destination.

Operations act on marked entries when present, otherwise entries in the explicit
visual selection, otherwise the current entry. Render the exact target count and
identities before application. Marks use typed entry identities and remain
visible; do not repurpose Vim `m` or Yazi's bare Space. Native Vim text yanks stay
text yanks outside the filename-editing/provenance contract in §7.

### Required supported envelope

| Operation | Local | Supported POSIX/Python SSH | SFTP-only / unsuitable host | Container |
| --- | --- | --- | --- | --- |
| List, parent, open, refresh | Required | Required via existing read path | Required where read permissions permit | Shared read-only view |
| Path completion | Required | Required | Existing SFTP completion remains usable | Use only supported read capability |
| Create regular file, exclusive | Required | Required protected helper operation | Explain missing safe mutation capability | Refused by policy |
| Create directory / reviewed parents | Required | Required protected helper operation | Same | Refused |
| Rename / same-filesystem move, no clobber | Required for regular files and directories | Required where backend proves no-replace semantics | Specific refusal, no unsafe emulation | Refused |
| Copy regular file within namespace | Required, including a different local mount | Required protected copy operation | Specific refusal | Refused |
| Trash | Native/platform-supported Trash; tested on the supported local platform | No portable remote Trash assumed | Unavailable, never permanent-delete fallback | Refused |
| Permanent file / empty-directory removal | Required, explicitly destructive | Required with revalidation and explicit confirmation | Specific refusal | Refused |
| Filename-edit draft | Required | Same model; only supported operations can apply | Draft can explain/refuse unavailable actions without mutation | No writable draft advertised |
| Search here | Existing local provider | Required fixed read-only rg provider when available | Named capability refusal | Do not invent an execution capability |

A capability describes operation, namespace/principal/incarnation, object kind and
safety guarantees—not one `can_write: bool`. Distinguish unsupported policy,
unavailable helper/platform feature, missing permission, conflict, busy state,
invalid path, incomplete observation and unconfirmed outcome. Absence of a safe
primitive is not a reason to fake success or shell out to `mv`/`rm`.

A remote directory's read-only buffer does not prohibit explicitly authorized
file operations. Conversely, `:remote edit` on one file does not grant mkdir,
rename, delete or overwrite authority over its parent. Operation admission is
separate and names the exact resources/plan the user approved.

## 6. Creation, rename/move, copy and removal semantics

### Creation is exclusive, not truncation

New file accepts a literal name or explicit destination. It creates an empty
regular file through an exclusive/no-clobber operation, then opens it after a
confirmed receipt. Source text is never taken from the directory listing. Cancel
before mutation leaves no target; cancellation after possible publication is
reconciled rather than reported as “nothing happened.”

New directory uses exclusive creation. Missing parents are not silently created:
show the complete proposed parent chain and require approval. Each newly created
component receives its own result; a later failure does not justify removing a
parent that another actor has populated. Ordinary `:e missing-file` remains the
separate deferred-until-save workflow.

Creation checks both filesystem destinations and already-open unsaved documents
with that target location. An empty draft buffer named `new.rs` is a real conflict
participant even if no on-disk `new.rs` exists. Resolve explicitly; never leave two
independent live documents accidentally owning one new resource.

Apply the documented creation mode/umask; private stages are private before any
content is written. Distinguish parent missing, permission denied, entry exists,
symlink/alias refusal and uncertain publication. Never use `exists()` followed by
ordinary truncating open as an exclusive-create implementation.

### Rename and move

Rename changes one entry's name; Move supplies a destination directory/location.
Both preserve the source document and bytes. Default is **no overwrite**. Show
source → destination, affected open descendants and any namespace/mount boundary
before applying. A single rename confirmation can be compact; it still uses the
same checked plan as a batch.

**`std::fs::rename` and `os.rename` are not no-clobber primitives.** Rust's API
explicitly permits replacing an existing destination. An existence check before
rename has a race; wrapping it in a worker or adding a dialog does not fix that.
Use an actual supported no-replace primitive, or refuse that operation on that
backend. An overwrite feature, if added later, needs a separate reviewed contract.
Do not use the existing remote save helper's `os.replace` as a generic rename.

A logical `Filesystem::Local` or one SSH endpoint can contain multiple mounts.
Same namespace is not proof of same filesystem. A move that receives EXDEV is
refused; do not silently implement it as copy+delete and call it atomic.
A separately chosen Copy remains available where supported.

Directory moves use native path-component/identity relationships, not string
prefix replacement (`a` must not match `ab`). Validate attempts to move a directory
inside itself, occupied targets, case-only rename behavior and active root/service
bindings. Ordinary directories with open descendants must be supported. Moving an
active workspace root, cwd anchor or transport working root may be specifically
refused until its broader root-relocation contract exists; that is not permission
to refuse every directory with an open buffer.

Symlink/hardlink mutation and aliased paths need explicit backend semantics.
Never dereference a selected link and mutate its target by accident. Browsing and
opening links remain usable; unsupported link-operation/alias cases are named
refusals. Do not claim canonical paths and logical resource locators are the same
identity: moving a path containing an alias can affect the spelling a document
uses even when its resolved target is outside the moved tree.

### Copy

Copy regular files to a unique destination through private staging and exclusive
publication. Bound memory and stream large content; capture and verify source
observations under the backend's stated concurrency model. Do not expose a
half-written destination under its final name. Display progress and terminal
per-item outcomes.

Copy normally means stored file bytes. If a source has unsaved editor changes,
require an explicit choice: **Copy stored file** or **Copy current buffer
contents**. The latter uses a pinned source snapshot and does not save or clean
the original buffer. Do not guess which version the user meant or silently save
before copying. Show the actual metadata preservation policy; do not imply ACL,
xattr, ownership, timestamp or set-id preservation that the backend does not
provide. Refuse unsupported security-sensitive metadata rather than guessing.

Same-namespace remote copy must be a real protected helper operation, not an
unbounded download/upload loop or a shell string. Cross-namespace transfers and
recursive directory copy remain a distinct extension envelope (§12).

### Trash and permanent removal

Trash is recoverable platform behavior, not a euphemism for unlink. Use an actual
supported platform Trash implementation and expose its recovery/reference result.
No Trash capability means no Trash action. In particular, do not invent a hidden
remote trash directory and claim desktop-trash interoperability.

Permanent deletion is a separate, clearly destructive action with explicit
confirmation. The initial supported envelope is files and empty directories;
non-empty recursive deletion needs the separate traversal/recovery contract in
§12. Local Trash can move a whole directory when its platform/backend and open-
descendant policy support it; that does not authorize recursive permanent delete.

Dirty/open sources require named handling before admission. Never discard dirty
text or silently close affected buffers. If deletion completes while newer edits
exist, preserve them in an explicitly detached/recovery document, revoke the old
write binding and offer restore or an explicitly named destination. A subsequent
`:w` must not silently recreate the deleted file from an obsolete binding.

Deleting a listing row creates a pending removal intent. Where Trash is
unavailable, require the user to choose permanent removal explicitly; do not
convert the intent to unlink behind their back. Protected active workspace/state
resources must be recognized by identity/policy, not substring matching, and
receive an explicit refusal or separately designed admission—not bulk destruction
because they happened to match a filter.

## 7. Modal filename editing, without filesystem effects per keystroke

`:fs edit` / Edit names enters a **filename draft** for the captured directory
snapshot. Keep the ordinary modal grammar. The editable text is names/relative
destinations; metadata and source identity are model data, not editable authority.
A name-only editable projection with metadata rendered beside it is preferable
to letting users accidentally edit sizes, permissions or hidden numeric IDs.

Required semantics:

- Edit an existing name: propose rename/move of that captured entry.
- Add a new row: propose creation; a new directory marker such as a trailing
  separator is interpreted only as explicit directory-creation syntax, not as
  permission to reinterpret an existing entry's type.
- `dd`/delete a complete entry: stage removal, with Trash/permanent disposition
  resolved under §6. Nothing is removed from disk by the text edit.
- Yank/paste a complete entry: retain structured source provenance and propose a
  copy once a distinct destination is supplied. Normal characterwise text paste
  continues to edit text. Plain external clipboard text is not proof of a file
  source and cannot authorize reading/copying an arbitrary path.
- Delete then paste the same entry within its draft retains entry identity rather
  than becoming an accidental delete-and-recreate. Cross-directory cut/move uses
  an explicit shared draft/operation identity; if not representable safely, direct
  the user to the supported Move action rather than guessing from identical text.
- `u`, redo, visual edits, macros and multiple selections change only the draft.
  They do not undo already committed filesystem operations.
- `:w` compiles the draft and opens the common **filesystem change review**.
  `:apply-change` performs the approved filesystem plan; `:cancel-change` returns
  to the intact draft. The labels state “Apply filesystem changes,” not “Save
  source buffers.” No alternate direct apply path exists for filename editing.

Stable per-entry tokens and register/transaction provenance belong out of band.
Do not infer source identity from the edited text, a visible row number, a label
prefix or a duplicate basename. Whole-line register metadata must not corrupt
ordinary text-register behavior when pasted into a source buffer. New duplicate
rows need their own draft identity and a distinct target; they cannot both own
one original entry. Joining/splitting rows in an ambiguous way produces located
draft diagnostics, not a guessed source/destination pairing.

Names are native paths. Lossy UTF-8 rendering is display only. Reuse a documented
lossless codec where editable native-name representation is available. If a name
cannot be safely round-tripped in the draft editor, protect that field and offer
the structured Rename action with an explicit new name; do not disable browsing
or operate on a lossy alias. Controls/newlines in names render visibly escaped,
never as terminal instructions or extra actionable rows.

Before any operation, diagnose empty names, duplicate destinations, invalid native
components, missing parents, unsupported entry kinds, parent/child overlap and
conflicting rename graphs. Never apply only the easy rows without the review
showing the refused portion. Acyclic batches can be dependency-ordered; cyclic
swaps require a designed private-staging/recovery strategy or an explicit
unsupported-graph refusal before mutation. The first implementation need not
pretend arbitrary rename graphs are one atomic transaction.

On confirmed application, reconcile to a fresh directory base and leave filename
edit mode. An old text undo stack must not silently recreate a draft against an
obsolete filesystem snapshot. Keep the operation receipt and recovery action
available independently. External refresh while a draft exists preserves the
draft and marks conflicts; it does not overwrite the user's proposed names.

## 8. Plan → review → apply → receipt → verify

The common review shell can host text and filesystem plans, but **filesystem
operations are not text edits to a generated buffer**. Preserve a typed plan kind
and a distinct executor. Share presentation, cancellation and receipt conventions,
not an abstraction that erases very different side effects.

### Minimum plan data

- An owned operation ID and requesting view/draft identity.
- Captured filesystem namespace/principal/connection incarnation.
- Ordered typed steps, exact native source/destination locations and object kinds.
- Before observations, required safety capabilities, collision/parent conditions.
- Affected document bindings, pending-save exclusions and source snapshots when
  copying live text. Text revision alone is not a resource-binding identity.
- Destructive/overwrite disposition and user-approved scope.
- Cancellation/verification information and a per-item result slot.

Pure planning may run on a worker for large drafts; all filesystem inspection,
capability probes, copying, helper execution and expensive revalidation are owned
jobs. Input→render does not wait. The review uses the same validated plan the
executor consumes; no independent pretty-printer that recomputes a different set
of actions from display rows.

### Operation lifetime is not popup lifetime

A stale **read** response may be ignored. A **mutation** receipt may not.

After admission, the workspace operation ledger owns the operation even if the
browser closes, the pane changes or the originating document is gone. Focus
freshness can suppress navigation/toasts, but cannot erase a confirmed rename,
lose a per-item result or leave live document paths bound to an obsolete location.
Reconciliation is idempotent by operation/step identity; UI publication is separate.

Cancellation before the side-effect boundary can report unchanged/cancelled.
After that boundary, a real committed receipt wins over a queued cancellation.
Connection/process loss or timeout with unknown commit status is **Unconfirmed**,
not Failed-before-mutation. Block conflicting operations on affected bindings
until verified. Graceful shutdown follows the existing remote Save/Verify drain
contract; explicit forced shutdown must name unresolved work. Do not promise crash
recovery without the requisite private durable intent/receipt record.

### Per-item outcomes and recovery

Use typed distinctions for committed, refused/conflict, failed before mutation,
cancelled before mutation, partial batch and unconfirmed. A batch is not atomic
merely because one rename is atomic. Show exactly which items changed, which did
not and which are unknown. Never auto-retry an uncertain mutation or “rollback”
by deleting a path that could now belong to another actor.

Verify compares observed state with both the before-state and intended outcome.
If the evidence cannot prove who created an existing destination, report that
uncertainty; observing the desired name is not authority to delete or overwrite
it. Keep dirty text and recovery data independent of the uncertain pathname.

Recovery actions are checked **new plans** derived from receipts: rename back,
restore from real Trash, or remove an unchanged created copy where supported.
Revalidate identity and vacant destinations. Native text `u` is not a promise of
filesystem undo, permanent deletion is not recoverable by assertion, and a
conflicting reverse operation must refuse rather than overwrite newer work.

## 9. Rename/move and the live editor

Resource relocation is the hardest shared boundary. Do not implement it by
closing/reopening buffers or assigning a new PathBuf and hoping existing workers
catch up.

### Admission

1. Find affected documents through typed namespace and path-component/binding
   identity. Include open descendants of a moved directory and open unsaved
   destination documents. Distinguish logical paths, resolved identities and
   aliases; no string-prefix or canonicalization-only shortcut.
2. Serialize against in-flight local saves, remote Save/Verify, pending edit
   admission/refresh and unresolved mutation attempts. A save must not recreate
   the old name after the move. Refuse/queue visibly; never silently lose its
   receipt or mutate whichever path is current when the worker finishes.
3. Ordinary dirty files and dirty descendants are supported for rename/move:
   preserve their live text, history, selections and DocumentId. No implicit save
   or forced reload. Their edits can continue while the operation runs; a newer
   text revision does not invalidate an otherwise confirmed path relocation.
4. Refuse explicitly when aliases, active-root relocation or backend observations
   cannot be reconciled safely. List the affected resources and how to resolve
   the blocker; a blanket “dirty buffer” refusal is not the required behavior.

### Confirmed relocation

- Retain DocumentId and the current rope/undo state. Update the resource binding,
  path/name, saved-file identity/baseline and binding epoch from confirmed data.
  Do not mark dirty text clean just because its filename changed.
- Revoke stale remote write permits. A permit bound to the old location is not
  transferable authority; use protected re-admission against the new binding
  before a later write. Preserve unsaved text throughout.
- Retire LSP/completion/diagnostic/preview requests tied to the old binding.
  Close/reattach under the new URI/root/language context as required. Path-derived
  language inference may change; explicit user language choices remain explicit.
  LSP symbol rename is not file rename. Automatic import refactoring through
  `workspace/willRenameFiles` is a separate extension, not faked by text search.
- Update source locations used by collections, review plans, jump/return records,
  MRU/session metadata and labels. Source content/history stay attached to the
  same document; a pending text plan pinned only to a revision must not survive
  a relevant binding change unnoticed.
- Invalidate/refresh affected parent Directory views, file catalogs, search hits,
  previews and Git/source-status data through their existing owned jobs. Preserve
  the relocated selection by identity. Old late results cannot resurrect the old
  path or steal focus in another pane.
- An unconfirmed outcome retains enough before/intended information to verify
  either location. Do not eagerly publish success, transfer permits, allow a
  save to the old path or discard dirty buffers while the binding is uncertain.

This applies equally to explicit Rename/Move and filename-draft application.
Keep one relocation reconciliation function/owner rather than duplicating it in
local, SSH, picker and directory key handlers.

## 10. Architecture and backend safety

### Shared data, small real adapters

Keep pure filesystem identity and listing metadata beside the 0042 resource
contract in `strop-workspace`: resolved directory location, native entry name,
entry kind, optional metadata, source observation and listing completeness.
Editor-specific draft IDs, DocumentIds, operation history and focus stay in the
engine; transport leases stay with the backend. No Ratatui types in these models.

Generalize `DocumentSource::RemoteDirectory` into one Directory source and migrate
all consumers. Replace endpoint-shaped parent/child keys with `ResourceLocation`.
A shared `DirectorySnapshot`/entry map drives the real buffer and renderer;
rendered names never become authoritative targets. A listing revision/entry token
owns selection and actions; refresh cannot make an old row number select a new
file accidentally.

Local uses native directory reads and platform file primitives on owned workers.
SSH uses the existing read-only SFTP reader and a deliberately extended fixed
protected-operation helper. Container adapters expose their existing read-only
operations and typed policy refusals. Avoid a universal provider/plugin framework
with dozens of speculative methods; implement the actual namespace dispatch and
capability data consumed here.

### Protected SSH operations are new work

0040 supplies the security/ownership pattern, not these operations. Extend the
framed protocol deliberately with typed create/mkdir/rename/move/copy/remove and
verification results. Split helper/protocol code by responsibility as necessary;
do not turn the current single-file save helper into a thousand-line opcode
switch or smuggle general shell execution into it.

Retain strict host identity, configured/versioned helper discovery, native-byte
arguments, bounded input/output, owned process lifetime and explicit admission.
Verify parent/source/destination bindings and no-follow/link policy; protect
cooperating operations with a deterministic lock ordering. Multiple-parent moves
need an actual lock/parent-validation design, not reuse of a same-directory save
assumption. Private stages and cleanup are tied to proven operation ownership.

Atomic destination no-clobber, source observation, cooperative locking, atomic
visibility and crash durability are different guarantees. Record what each
backend provides. Preserve 0040's explicit limit: advisory locks and final checks
do not provide universal CAS against noncooperating writers. Do not claim SFTP or
a helper eliminates that race. An environment requiring stronger guarantees must
receive a refusal or a separately designed server/filesystem mechanism.

Do not weaken 0040's ordinary remote save policy, readonly defaults or container
policy as a shortcut. New directory capabilities do not authorize remote save-as,
elevation, arbitrary commands or container writes.

### Bounds, replay and module ownership

- Bound actual outstanding listing/preview/search/helper work and retained
  listings. Large directories stream or publish bounded snapshots; sorting,
  metadata lookup and text projection do not happen over the whole tree per key.
- Completeness is visible. A limited/error prefix is not an empty/full directory;
  Select all means the shown, identified scope, never undiscovered entries.
- Cancel stale reads; keep admitted mutation outcomes until reconciled. Replay
  records/injects owned requests and outcomes and never invokes native mutations.
- Use existing modules as anchors: `editor/document`, `editor/io`,
  `editor/remote/directory`, `remote_completion`, `editor/changes`,
  `strop-remote/{transport,save,exec}`, `strop-containers` and buffer/picker render
  components. Migrate callers through symbol references, not guessed grep renames.
- Likely engine split: directory/view model, path input, drafts, operation planning,
  admission/apply, relocation, outcomes/recovery. Keep backend execution separate.
  Exact filenames follow the final 0051 layout; do not add all of this to
  `editor/io.rs`, `normal/ex.rs` or `document/remote.rs`.

A future GUI can consume these models/actions and implement its own rendering,
pointer interaction and drag/drop intent. No GUI dependency, embedded terminal,
Yazi subprocess or shell cwd synchronization is needed to make this feature work.

## 11. Implementation sequence and proof

### Dependency order, not reduced release scope

1. **Identity/listing cutover:** shared Directory snapshots and documents; local
   listing worker; SSH/container migration; consistent entry paths and returns.
2. **Navigation/presentation:** local+remote path completion, current-file reveal,
   filter/visibility, source preview, responsive visuals and search scope bridge.
3. **Operation foundation:** capability inventory, exclusive create/mkdir,
   no-clobber rename/move, checked review, mutation ledger and live binding
   reconciliation. This is the shared prerequisite for every writing surface.
4. **Full requested operations:** protected copy/removal/Trash and recovery;
   supported remote executions and explicit unavailable-host behavior.
5. **Modal filename drafts:** provenance-aware rename/create/remove/copy, ordinary
   text undo, review/apply using the same operation foundation; no direct I/O.
6. **Integrated evidence and cleanup:** complete F ledger, user docs/changelog,
   remove obsolete RemoteDirectory/duplicate paths and temporary harnesses, run
   the complete repository gate once the integrated work is ready.

Independent backend, presentation and draft slices can run concurrently only
with explicit shared request/result/identity contracts. One integration owner
owns relocation and final admission boundaries. Do not make each slice invent
its own capability enum, operation receipt or path parser.

### Required observable cases

Use isolated temporary trees, private HOME/XDG locations and deterministic fake
transport/worker seams. Keep tests for real ambiguity, races and state transitions,
not for field forwarding or exact English labels.

- CLI directory, `:e directory`, `:browse`, Space-e reveal, parent/child and file
  round trips resolve to the same model locally and over SSH. Dirty origin views
  are not lost. No global cwd change during browsing.
- Literal spaces, colons, leading dashes, Unicode, duplicate basenames, controls
  and non-UTF-8 native names never alias another file or execute as shell text.
  A symlink is identified honestly, not accidentally treated as its target for
  mutation. Remote/container identities cannot fall back to local paths.
- Local and supported SSH create/mkdir succeed; occupied paths and a concurrently
  created destination remain untouched. Missing-parent approval is explicit.
  Open unsaved destination buffers are accounted for before any mutation.
- Rename an open dirty file and an ordinary directory containing open dirty
  descendants. Text, DocumentIds, undo and selection survive; later save writes
  the new binding. Save-in-flight and unconfirmed-save races cannot recreate an
  old path. LSP/collection/history consumers follow confirmed relocation.
- Occupied destination, self-descendant move, EXDEV, case-only/native identity
  conditions, unsupported alias/link and active-root cases receive accurate
  outcomes. No `exists`/rename race overwrites an unrelated destination.
- Copy stored bytes versus dirty buffer contents produces the explicitly chosen
  version. Failed/cancelled copy does not expose a half-file or delete another
  actor's destination. Metadata guarantees match the receipt.
- Trash really trashes and can offer checked restore; permanent deletion requires
  its own intent. Unsupported remote Trash does not become unlink. Non-empty
  recursive deletion is refused under the bounded envelope.
- Edit names with ordinary motions, visual edits, `dd`, `yy`/`p`, undo/redo and
  macros. Disk remains unchanged until Apply. Identity survives text edits;
  duplicate/ambiguous/conflicting drafts are diagnosed before mutation.
- Closing the browser/changing panes after admission does not drop committed
  outcomes. Lost SSH acknowledgements, queued cancellation and partial batches
  retain exact per-item evidence and verify without automatic retries.
- Directory refresh during a draft, reordering, deleted selected entries, stale
  completion/preview results and listing limits preserve identity and honest
  completeness. Large work does not block the input/render loop.
- Real Search-here captures the correct root/namespace on local and capable SSH;
  source opening/collection promotion stays in that namespace. Missing remote rg
  is a capability refusal, not a local or full-download fallback.
- Containers remain genuinely read-only through commands, drafts and replay.
  No `docker cp` write workaround or changed remote-save authorization slips in.

Run actual TUI workflows and capture 140×40, 100×30, 80×24 and resize/tiny recovery:
local navigation → create → rename → copy → inspect receipt; SSH equivalents on
the disposable supported host; filename editing/review/cancel/apply; failure and
unconfirmed outcomes. Verify real filesystem bytes/names and live buffer state,
not just rendered success messages. Golden cell-grid checks protect meaningful
identity/selection/diff/background distinctions. A backend mock is not proof of
remote operation support.

Finally run `docker compose run --build --rm test`, including the repository's
fmt, locked all-target clippy and locked tests. Update command help and relevant
existing docs/changelog. Do not run this gate against another session's partially
edited source just to validate the present documentation-only handoff.

## 12. Explicit extension envelope and terminal exploration

These are **follow-on design boundaries**, not permission to remove F01–F12.
If implementation moves a required behavior here, user approval and the 0028
ledger are mandatory. Some are older ambitions with their own plans; do not claim
this milestone delivers them merely because its UI can show an action name.

| Extension | Reason / re-entry gate |
| --- | --- |
| Recursive directory copy and permanent recursive delete | Need a bounded traversal/descendant policy, partial recovery and mount/link rules. Single-entry directory rename and supported local Trash are not this operation. |
| Cross-namespace and cross-device Move via copy+delete | Transfer ownership, cancellation/resume, content verification and two-sided recovery. Never an implicit rename fallback. Same-namespace regular-file Copy remains required. |
| Cyclic bulk rename/private staging | Design recoverable temporary-name scheduling and visible partial outcomes before advertising arbitrary rename graphs. Ordinary modal rename drafts remain required. |
| Link creation/retargeting, chmod/chown and richer metadata editing | Native object semantics and authority differ from changing a name. Preserve unsupported cases explicitly; do not execute metadata text edits. |
| Active workspace/root relocation | Registry, running service roots, session/trust/git/LSP and path aliases need a broader coordinated relocation contract. Ordinary directory moves remain required. |
| Semantic LSP file-rename refactoring | `workspace/willRenameFiles`/`didRenameFiles`, returned edits, cancellation and FS/text commit ordering need a contract. Basic binding close/reattach is required now. |
| Remote Trash and project-wide remote replacement | No portable Trash standard or blanket multi-document write authority is supplied by SFTP. Require a real backend/admission design; read-only SSH Search-here remains required. |
| Persistent named file-operation sessions/history | Requires private durable metadata and recovery semantics; do not promise crash restoration from an in-memory receipt buffer. |
| GUI / embedded terminal | [0055](0055-embedded-terminal-tui-and-gui.md) is the separate requested handoff: TUI first, GUI integration next when GUI work resumes. Neither blocks filesystem workflows or current TUI quality work. |

### Embedded terminal handoff — 0055

The canonical component research, architecture and delivery contract now live in
[0055 — embedded terminal: TUI first, shared GUI integration later](0055-embedded-terminal-tui-and-gui.md).
The user explicitly requested this order; do not wait for GUI work to make a TUI
terminal useful, and do not implement two independent terminal cores.

0055 compares published Alacritty, Zed's internal terminal components, WezTerm's
emulator and portable-pty, libghostty-vt, libvterm and smaller TUI building blocks.
Its preflight owns emulator/input/PTY/static-package evidence; T01–T10 own the
usable TUI feature and G01–G07 the later native GUI surface. No terminal library
has been selected irrevocably or integrated by this filesystem handoff.

Filesystem operations remain native structured operations, not commands typed
into an embedded shell. A later “Terminal here” action passes a captured location
and explicit launch authority into 0055's service; remote interactive execution
does not inherit authority from a protected file-operation helper.

## 13. Primary references and what they justify

- [GNU Emacs Dired](https://www.gnu.org/software/emacs/manual/html_node/emacs/Dired.html):
  real directory buffers, local and remote filesystems.
- [Dired operations](https://www.gnu.org/software/emacs/manual/html_node/emacs/Operating-on-Files.html):
  marks, explicit destination/confirmation, copy/rename/delete, and updating
  visited buffer filenames after rename.
- [Wdired](https://www.gnu.org/software/emacs/manual/html_node/emacs/Wdired.html):
  explicit writable filename mode, normal text editing, commit and protected
  metadata. Its choices are precedent, not permission for unsafe implicit deletes.
- [Oil.nvim](https://github.com/stevearc/oil.nvim): directory-as-buffer, parent/open,
  edit then write/confirm, and LSP file-method considerations.
- [Yazi quick start](https://yazi-rs.github.io/docs/quick-start/): navigation,
  selection, preview, create/rename, distinct Trash/permanent deletion and tasks.
- [Rust `std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html):
  destination replacement and mount-boundary behavior; why it is not no-clobber.
- [0040](0040-remote-editing-and-saving.md): current protected helper authority,
  cooperative concurrency limits and Save/Verify lifetime guarantees.
- [Neovim terminal documentation](https://neovim.io/doc/user/terminal.html):
  actual embedded terminal/PTY/input-mode behavior, not an absent feature.
- [Ghostty project](https://github.com/ghostty-org/ghostty) and
  [libghostty-vt API](https://libghostty.tip.ghostty.org/): current embedding scope
  and explicit unstable-API warning. Recheck maturity at the later evaluation;
  do not treat this handoff's research snapshot as a permanent compatibility claim.
