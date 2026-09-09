# 0040 — Explicit remote editing and conflict-aware saving

Status: implemented and container-verified for 0.19.0, the release after 0.18.0.
This implements 0035 RW4, not directory mutations, arbitrary remote commands,
elevation or a writable mount. Read-only remains the default.

## 1. Evidence and the concurrency boundary

OpenSSH's [POSIX rename extension](https://github.com/openssh/openssh-portable/blob/master/PROTOCOL)
performs `rename(oldpath, newpath)`; its fsync extension syncs an open file handle.
Neither supplies atomic compare-and-swap against a content version. A client-side
hash check followed by SFTP rename therefore cannot exclude a racing writer.

POSIX replacement is atomic within one filesystem; [Python's os.replace](https://docs.python.org/3/library/os.html#os.replace)
exposes that operation with directory-descriptor-relative paths. Atomic visibility
is not crash durability: stage and parent directory both need fsync.
[flock](https://man7.org/linux/man-pages/man2/flock.2.html) is advisory and tied to
an open file description. It excludes cooperating writers, not every program with
filesystem permissions. Network filesystem lock semantics are not universal.

Decision: use a small built-in Python helper under the existing owned SSH execution
supervisor. It holds a stable per-file cooperative lock while checking the baseline,
preparing metadata and replacing the file. Strop remote-save protocol participants
on the same effective lock domain cooperate; local strop saves and other programs
are nonparticipants. Their completed changes are detected by content and metadata
comparison before commit. **There is no claim of race-free exclusion
against a noncooperating writer changing the path in the final check/rename window.**
The UI/documentation must name that limit. An environment requiring universal CAS
needs a server/filesystem versioning service; strop does not pretend SFTP provides it.

Relevant existing boundaries:

- `strop-remote::exec` already owns noninteractive SSH, strict host keys, native-byte
  argv, bounded output, versioned/configured Python discovery and lease cancellation.
- `strop-core::process::capture_with` owns process groups, deadlines and concurrent
  bounded pipe drains. Its held-stdin mode keeps the remote lifetime lease open.
- `editor::io` registers document/revision/focus-owned saves and accepts only their
  matching receipts. Remote documents deliberately have no local `Buffer.path`.
- Remote snapshots are excluded from persisted sessions. Replay starts before remote
  service admission and injects recorded requests/results without native execution.

## 2. User-visible behavior

1. Open/browse a remote file as today. It is read-only.
2. Run `:remote edit` on a complete, non-following regular-file snapshot. Admission
   verifies the displayed bytes against the server and captures a write baseline.
   Success grants this document incarnation a writable permit; insert/operators,
   undo/redo and ordinary `:w` then operate on that same real buffer.
3. `:w` saves a frozen revision asynchronously. Typing continues. A receipt clears
   dirty state only for the exact written revision; newer edits remain dirty.
   `:wq` closes only after confirmed saving and the existing focus checks.
4. `:w!` does not bypass remote conflict checks. Save-as, directory mutations,
   symlink/hard-link writes, partial-window writes and Git mutations remain refused.
   There is never an implicit local-path fallback.
5. A conflict preserves both remote contents and local edits. Refresh/reopen is an
   explicit user decision. Failed metadata preservation never becomes success.
6. If cancellation/connection loss occurs after admission, commit may be unconfirmed.
   Keep local edits dirty and report that uncertainty, not "unchanged" or "written".
   `:remote verify` explicitly reconciles the last attempt: compare the server with
   the before fingerprint and the intended bytes. Matching intended bytes can confirm
   the saved revision; unchanged baseline permits an explicit retry; any third state
   is a conflict. No automatic overwrite, retry or rollback.
7. Successful refresh or closing/reopening revokes the permit; a new `:remote edit`
   is required. Reselecting an already-open file preserves its document and edits.
   Pending/unconfirmed saves block refresh; pending refresh blocks save/admission.
   A failed refresh keeps the existing edits and permit. No write permission,
   native connection or pending write is restored from session files.
8. Edit admission and following are mutually exclusive, including pending
   admission. Escape revokes a pending grant immediately, even if success is queued.
   Save/Verify retain their ticket until the real outcome is known; a queued durable
   receipt cannot be turned into a false cancellation. Graceful shutdown drains
   accepted Save/Verify work; explicit forced close revokes the document's authority.

`:remote edit` authorizes the editor's fixed helper, not project-defined code.
Project language-server configuration still uses the existing endpoint/workspace
trust gate. SFTP-only servers remain readable; editing requires the documented POSIX
and Python capabilities and otherwise refuses with a useful diagnosis.

## 3. Transport and transaction

### Typed ownership

The remote crate owns a checked `ContentDigest` (SHA-256), a serializable
`RemoteVersion` binding fingerprint/metadata to one `RemoteFile`, typed refusal/error
variants, and a committed receipt. File identity, content identity, mode and timestamps
are separate domains. Constructors/deserialization validate wire values; callers
cannot pair an unrelated file path with a baseline through parallel arguments.

Editor admission/save/verification tickets include document incarnation, file,
permit identity, captured buffer revision and relevant focus intent. Register before
launch. A stale result cannot grant editability, clear dirty state, change another
buffer's baseline, or close a different pane.
Retain the attempt's frozen rope, revision, before version and permit identity in
editor-owned state before launch. A digest can be derived from that immutable rope
on a worker; cancellation must not erase the evidence needed for verification.

### Admission

- Hash the frozen rope on a worker. Obtain matching server content plus metadata
  under the cooperative lock; a same-size change is still a conflict.
- Open the parent directory and target through native-byte, no-follow descriptor
  operations. Symlinked ancestor directories (the NFS-home norm on enterprise
  hosts) are resolved component-wise and the no-follow walk restarts from the
  root, bounded at 40 hops; every opened ancestor remains a revalidated real
  directory. A symlink as the FINAL component, nonregular targets and multiple
  hard links stay refused.
- Reserve the protocol's lock and transaction prefixes in every native path
  component. Ordinary remote edits/saves must never replace an active lock inode
  or edit another transaction's stage. Validate lock path/descriptor identity,
  regular-file type, single-link count, owner and private permissions.
- Require exact stored native component spelling before deriving the lock key.
  Case/normalization aliases must not create distinct locks for one destination;
  refuse noncanonical spellings and offer reopening through the directory browser.
  Apply the reserved-namespace check to those stored names, including ancestors.
- Check actual access through a nontruncating read/write open. Require ownership and
  capabilities needed to preserve metadata; do not silently change the owner.
- Capture device/inode, size, content digest, mode, uid/gid, nanosecond mtime/ctime
  and an extended-attribute fingerprint. A stable read checks descriptor and path
  identity before/after hashing. Use the existing 256 MiB complete-snapshot bound.

### Commit

1. Acquire a nonblocking exclusive advisory lock on a same-directory, private,
   no-follow, owner-checked lock file keyed by the native basename. Its inode is
   stable: do not unlink it on release and accidentally create two lock domains.
   Competing strop saves report busy rather than waiting on the input thread.
2. Reopen/validate the destination and compare its complete baseline, including
   content and metadata. Missing/replaced/changed destinations are conflicts.
3. Create an exclusive unpredictable **0700 transaction directory** beneath the
   destination's parent, then an exclusive private stage inside it. Verify the
   directory's effective privacy before receiving data. Transfer exact frozen bytes
   through a length-framed stdin payload, never argv or shell text. Keep the SSH
   stdin writer open after transfer: EOF means lease loss, not successful upload.
   Bound the edited payload again at save time, independently of output limits.
4. Restore owner/group, permission bits, extended attributes and original mtime
   inside the protected directory. Capture actual attribute names/values while
   checking their fingerprint; a digest alone cannot restore them. Restore ownership
   before mode/ACL state, account for inherited stage attributes, and verify the
   complete resulting metadata. Missing/inaccessible preservation is a refusal.
   Inode and ctime are comparison fields, not values that can be restored.
5. Fsync the staged file and transaction directory. Recheck the original baseline,
   requested path bindings and lock identity immediately before commit while holding
   the cooperative lock. Observe cancellation before crossing the commit boundary.
6. Rename through held source/destination directory descriptors, from the private
   directory into its parent on the same filesystem. Fsync both changed directories
   before returning a committed fingerprint/metadata receipt. A post-rename failure
   is explicitly unconfirmed, never a precommit failure claim.
7. Cleanup removes only this transaction's stage and directory, then syncs the
   parent as required. Install termination handling before creating staging state:
   Python `finally` alone does not handle the supervisor's default SIGTERM action.
   On NFS, renaming/unlinking an open file silly-renames it to `.nfsXXXX` until
   the last client handle closes, so cleanup closes the stage handle before
   unlinking and tolerates transient ENOTEMPTY on the stage rmdir with a bounded
   retry; a still-failing cleanup after a committed rename stays explicitly
   unconfirmed, never a false precommit failure.
   Observed cancellation/errors attempt cleanup; SIGKILL, host death or expiry of
   the supervisor's TERM grace may leave an orphan. Its 0700 parent protects it even
   after final file modes/ACLs were restored. Never glob-delete unrelated files.

The helper runs under the interpreter already selected by the supervisor, including
versioned-only installations and `STROP_REMOTE_PYTHON`, with isolated/no-site startup;
do not introduce a hidden `python3` assumption. The helper is shipped source, not a
remote daemon installation. Use a harmless fixed supervisor cwd, then walk from a
trusted root descriptor; generic supervisor `chdir` is not no-follow validation.
The native-byte protocol has fixed versioning and separately bounded inputs/errors.

### Cancellation and verification

The existing supervisor lease remains useful, but its documented limitations remain:
partitions delay disconnect detection; supervisor SIGKILL cannot guarantee cleanup.
Before rename, a stopped transfer/helper cannot partially overwrite the original.
After rename starts, cancellation can race with commit: the receipt, or explicit
verification, determines the visible result. Never attempt an unsafe blind rollback.

Verification compares under the same lock; it cannot replace an unexpected state.
Unchanged-before requires the complete original fingerprint. Intended-state requires
the intended bytes, preservable expected metadata and a regular single-link target;
it does not require the old inode or ctime, which replacement changes. Before durable
acknowledgment, fsync the observed file and parent directory and capture its new full
version as the baseline. Equivalence does not prove historical authorship or that an
old helper terminated. Unchanged-before permits a new explicit conditional attempt,
not blind retransmission. Unrelated content/metadata remains conflicted. Saved
revision and newer dirty edits are distinguished throughout.

## 4. Implementation slices and gate

- Remote transport: a held-input capture path, fixed-helper execution using the
  selected interpreter, typed save protocol, metadata/conflict/atomicity handling.
- Editor: per-document permit/baseline and attempt state; owned edit/save/verify
  events; `:remote edit`, `:remote verify`, `:w`/`:wq` routing; refresh/close revocation,
  readonly derivation, help and replay. Keep local save semantics unchanged.
- Verification: bounded save-state TLA+ model with explicit cooperating-writer
  assumption, cancellation/crash/receipt-loss transitions, progress qualification,
  reachable success/conflict witnesses and deliberate faulty variants.
- Real SSH oracles: edit/save/reopen; same-size external changes; competing saves;
  interrupted stage; permissions/mtime/attributes; symlink/hard-link refusal;
  postcommit lost receipt and explicit verification; newer local edits during saving;
  non-UTF-8 filenames; configured/versioned Python; no local lookalike mutation.
- Actual TUI/headless editing and full replay demonstrate the user path. Existing
  save/focus/trust/readonly suites follow the changed contract; no source-text tests.
- Docker quality and protocol gates, hosted CI, all release artifacts and the public
  website must pass before the separately versioned remote-editing release is done.

The proof is bounded and assumption-qualified. We do not advertise universal
filesystem CAS, arbitrary-writer exclusion, remote Windows support, or cleanup after
an unobservable host/supervisor death.

## 5. Verification record

Independent static security reviews covered both the helper/transport and editor
ownership paths. The design was strengthened with reserved control-path ancestors,
protected staging, and durable verification. The editor review found pending
admission/follow, queued-grant cancellation and `:wq` operand loss; their source
guards and deterministic consumer-facing regressions are now implemented and pass.

The real editor completed SSH edit/save/refresh with an exact saved revision,
then replayed the full trace without native I/O. The configured versioned
`/fixture/remote-bin/python3.11` interpreter also completed a real save.

The complete compose model gate passed with `RemoteSave.tla` added. Its primary
adversarial one-participant configuration explored 1,023 states, its two-participant
cooperation configuration 411, and its qualified progress configuration 41.
Eight deliberate faults were rejected; success, confirmation, cancelled commit,
private orphan and verification witnesses were reached. A separate nonparticipant
counterexample demonstrates the expressly unclaimed universal-CAS guarantee.

Filesystem/owned-process tests exercise metadata and attributes, symlink/hard-link
and namespace refusal, stage interruption, competing locks, lost receipts and
stdin-lease delivery. The real SSH suite interposes the fixture's rename syscall
before/after replacement while running the unchanged shipped helper. This is fault
injection in the test environment, not a special production or demo code path.
All workspace/container checks passed with SSH required, including the real
same-size conflict/save-as refusal, native-filename save/replay and pre/post-rename
SSH crash oracles. The actual terminal editor also saved `VERSIONED TUI` through
SSH, retaining mode 0640 and the original nanosecond mtime, then exited normally.
