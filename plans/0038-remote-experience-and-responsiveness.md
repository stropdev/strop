# 0038 — Remote opening, truthful directory views, and a responsive editor

Status: implemented and container-verified for 0.18.0. This includes the user's
follow-up requests about indentation guides and a GUI recommendation.
The reported stall is in **local** workspace file search. Responsiveness work covers
every keystroke and the complete input-to-render path, not just either file finder.
The user also requested a new release: target 0.18.0 after container and hosted CI
gates, meaningful release notes, artifact verification and live website deployment.
The later user request adds CMake/Markdown and the compatible rootle language
coverage to this release (0039). Remote-file writes are explicitly the next
implementation/release after this one; RW4's safety gate is not waived.

## 1. Priorities and deliverables

| Priority | Work in this change | Observable completion |
|---|---|---|
| P1 | Eliminate interactive-thread blocking and unbounded publication work | File search stays usable while a large workspace streams and ranks; Escape, typing, rendering and pane changes continue promptly |
| P1 | Restore headless access to project trust | Explicit `:trust` and existing persisted trust work with isolated state roots; the project-command gate remains enforced |
| P1 | Support enterprise remote Python layouts | Git/LSP work with a versioned Python on PATH or an explicitly configured absolute interpreter; SFTP browsing remains Python-independent |
| P2 | In-editor remote opener | Choose/add a host, connect explicitly, then browse real directory buffers without constructing an SSH URI manually |
| P2 | Remote permissions, sizes and file-kind distinction | Structured SFTP metadata reaches the directory surface; missing fields are shown as unknown, never fabricated |
| P2 | Structural indentation guides | Guides express nesting/scope changes, not every whitespace tab stop or alignment column |
| P2 | Discoverable headless scripting and bounded settling | `--help` lists the actual directives; `settle [MS]` returns when work drains and fails on timeout; `wait MS` retains delay semantics |
| P2 | Accurate usage and release notes | Local/headless file locations and remote directories are advertised; 0.16.0/0.17.0 GitHub releases contain their changelog sections |
| P2 | Expanded static syntax coverage | CMake, Markdown block/inline/injections and the compatible rootle additions ship under 0039 |
| P3 research, now | GUI verdict and stack recommendation | A sourced recommendation with an explicit go/no-go boundary; no GUI implementation or mandatory GUI dependency |

## 2. Research and product decisions

### Remote opening

[Zed's remote-development flow](https://zed.dev/docs/remote-development) separates
server selection/connection from choosing a remote path. Adopt that separation,
not its remote-server deployment or broad SSH option editor. Zed explicitly warns
about opening `/` or `~` as huge recursively indexed workspaces: our root browser
must enumerate only the selected directory, not recursively index the machine.

[Dired's manual](https://www.gnu.org/software/emacs/manual/html_node/emacs/Dired.html)
([upstream manual source](https://github.com/emacs-mirror/emacs/blob/master/doc/emacs/dired.texi))
uses an ordinary read-only buffer, minibuffer completion/history, entry navigation
and parent navigation that preserves location. Adopt those properties. Do not copy
its `ls` text parsing or its filename/newline ambiguity.

[Oil](https://github.com/stevearc/oil.nvim) provides Enter-to-select, `-` for parent,
file-kind ordering, configurable permissions/size columns and normal-buffer
interaction. Adopt the compact presentation and navigation, not writable directory
editing: remote writes remain behind 0035 RW4's conflict/atomicity gate.

Concrete flow:

1. `Space o` and `:remote` open the remote chooser. Reuse the editor's modal picker
   and its command/help registry, not a new widget with different input rules.
2. Offer known SSH hosts, connected hosts and bounded remembered destinations,
   plus an explicit new-host entry. Accept `host`, `user@host[:port]` and canonical
   endpoint spelling through the existing checked endpoint parser. Never interpret
   this field as a shell command or accept passwords in it.
3. Enter is the explicit connection action. Show owned progress; Escape cancels;
   failure leaves an actionable diagnosis and preserves the current document.
4. A connected destination opens its remembered directory, or the remote root for
   a new host. This is a real SFTP directory buffer, not an OS mount and not an
   implicit workspace-wide walk. Home and direct-path navigation remain explicit
   shortcuts; home expansion must obey the negotiated server capability.
5. Enter descends/opens, `-` visits the parent, `/` searches, and the existing
   `:filter` narrows the directory. Returning to a parent restores the selected
   child where possible. Root, home and refresh actions are discoverable in help.
6. Remember only successful endpoint/path choices in bounded private metadata,
   asynchronously. Do not persist credentials, remote contents, pending failures
   or automatic reconnect authority. Existing SSH config remains authoritative.

### Directory presentation

Preserve structured attributes from SFTP READDIR/STAT instead of parsing longname
strings or running `ls`. Introduce checked, named permission/size/type values at the
wire boundary. A missing attribute is distinct from permission `000` or size zero.
Distinguish directories, regular files, links and other entries when the server
actually provides the type; never invent a link target or recursively compute a
directory's aggregate size.

Use a compact type/permissions/size/name layout, aligned columns and restrained
file-kind colors with a non-color distinction. It must work without a special icon
font. Keep one buffer row per entry even for control characters/native-byte names;
entry actions use stable typed identity, never a path parsed back from rendered
text. Metadata and directory text are prepared on a worker. Rendering is bounded
to the viewport, and cursor/horizontal scrolling must agree with the displayed
columns. Preserve normal selection/yank behavior rather than special-casing grammar.

### Indentation guides

[Zed's guide code](https://github.com/zed-industries/zed/blob/main/crates/editor/src/indent_guides.rs)
operates on guide spans and an enclosing indented range; it refreshes active-scope
work separately. [VS Code's model](https://github.com/microsoft/vscode/blob/main/src/vs/editor/common/model/guidesTextModelPart.ts)
also treats scope beginnings/endings and whitespace-only lines explicitly. These
are not equivalent to stamping a marker at every tab stop on each line.

Decision: open rails from actual nesting transitions and carry them only through
their enclosing scope. An irregular alignment jump must not invent intermediate
rails, and uniformly indented text without a scope opener must not acquire them.
Do not simply hide rails on every equal-indented line: that would sever the useful
continuity inside a real block. Blank lines inherit only a surrounding live scope.
Keep scope computation outside rendering; do not copy Zed's foreground timed wait.

Implement a single revision-aware guide model with explicit opening/closing scope
boundaries and display-cell columns. Use language structure where the existing
syntax information supports it, with a conservative indentation-based rule for
plain text. Do not add a whole-document scan on every frame. Tabs, irregular indent
steps, continuations, blank lines, dedents, split panes and horizontal scrolling
must agree. Generated listings/help/diff metadata are not code indentation.
The existing `indent_guides = false` remains the off switch.

## 3. Responsiveness: fix the source, not the runtime label

Initial source evidence already explains a substantial stall path:

- `strop-picker::Picker::refilter` clones every matching row's text, allocates match
  columns and sorts the entire result set synchronously.
- `Picker::append` invokes that full refilter for every arriving source batch.
- `Editor::handle_picker_event` calls append on the editor thread. A streamed walk
  therefore repeats whole-catalog work on the input/render event loop.
- Headless `Driver::drain` drains until an unbounded producer queue becomes empty.
  The live event driver and other drains require the same fairness audit.

The release-mode existing scorebench measured one synchronous refilter at 2.156 ms
for 10,000 items, 11.017 ms for 50,000 and 21.387 ms for 100,000 (median of seven).
That excludes drawing and the queue of source batches. It diagnoses work placement,
not disk latency, and is not the final input-latency measurement.

Tokio already exists for remote/LSP I/O. Adding more Tokio does not move synchronous
CPU work off the UI thread and does not fix an unbounded queue. Reuse the owned
worker/event protocol. Evaluate the full nucleo worker engine against a dedicated
ranking worker, reusing current matching semantics; the chosen implementation must
own scanning/scoring/sorting away from the interactive thread, not start one thread
or process per keystroke.

Implementation decision: retain nucleo-matcher's pure matching rules, parse a
pattern once per pass, and run ranking in one owned actor per picker. A persistent
catalog of shared immutable items makes a request snapshot constant-time; ranked
rows carry indices plus flat match-column storage, not duplicated strings. The
renderer must borrow only visible rows, never clone the entire ranking.

The event transport gains a priority input lane and a background lane sharing the
same AppEvent reducer and recording boundary. Senders wake the receiving thread;
the live driver merges lanes with a bounded event/time slice before drawing.
This refines 0018's unified-event design without introducing a second input machine
or a per-source polling loop. Headless draining receives the same bounded budget.

Grammar reads above the bounded inline source/count budget use a persistent CPU
owner. Complete commands, counts and macros retain typeahead ordering; the same
pure resolver supplies execution and preview. One queued input per delivery keeps
full replay independent of host timing. Ctrl-C revokes queued work; Escape remains
an ordered grammar key once a command has been accepted. Search-prompt cancellation
still restores its complete origin and revokes preview work.

Long-line layout uses sparse immutable grapheme checkpoints. Workers build the
indexes; the buffer invalidates only proven prefixes through the mutation gateway.
Unprepared distant geometry is explicitly pending rather than a full prefix scan
inside painting. Search counts stream through the shared query engine and retain
only viewport hits. LSP request text is an owned rope slice, not a materialized line.

Required cutover:

- Results carry picker/source/query ownership. Superseded work cannot publish or
  overwrite selection. Closing a picker revokes its work immediately.
- Publish immutable ranked snapshots or bounded deltas. Do not clone full text,
  compute highlight positions for every invisible row, or re-rank the accumulated
  catalog on each batch. No silent result cap that hides valid files.
- Bound both producer backlog and editor publication per turn. Backpressure may
  block workers, never input. Coalesce superseded results and wake the event loop;
  input and rendering cannot starve behind a continuous result stream.
- Audit local/remote directory preparation, picker accept/preview, syntax parsing,
  buffer search/preview, Git/LSP completion, clipboard, close/cancel/destructors and
  render-time scans. Remove native I/O, lock/join waits and unbounded scans/copies
  from these paths. Merely putting a blocking call inside an `async fn` is not a fix.
- Heavy work uses frozen ropes/typed identities and validates freshness when it
  lands. Preview and execution keep the same grammar resolver; no preview-only
  approximation, warning suppression or stale-result fallback.
- Record an ownership/work table and measured before/after scenarios while
  implementing. Findings against the shipped nonblocking contract are P1 and must
  be fixed here, not silently deferred as future optimization.

Verification uses large real directory trees and synthetic large catalogs while
feeding navigation, query edits and Escape into the actual editor. Measure input
latency and publication work separately from disk/network latency. Targets on the
measured fixture are p95 within a frame (~16 ms) and no long UI stalls; report the
actual numbers and environment. CI regression oracles assert bounded work,
freshness, cancellation and fairness rather than fragile wall-clock percentages.

## 4. Reviewer feedback: complete fixes

### Remote Python

The supervisor currently hardcodes `exec python3`. Replace that with one shared,
bounded interpreter selection boundary used by both Git and LSP. Honor an explicit
`STROP_REMOTE_PYTHON` program/path (including `/opt/bb/bin/python3.11`), and otherwise
probe compatible `python3`/versioned Python names on the remote PATH. Validate the
required version/capabilities, quote the program as inert data, and preserve native
argv/cwd. An invalid explicit override fails rather than choosing another program.
Do not hardcode a particular employer's installation prefix. SFTP directory/file
browsing must not invoke Python just to become usable. Keep launch/cancellation and
supervisor ownership guarantees intact and update the associated model boundary if
bootstrap admission changes.

### Headless scripts and trust

`settle` already exists in the current source with a 30-second limit; it is hidden
and its timeout/error/drain behavior needs repair. Preserve this mechanism and make
it discoverable instead of inventing a competing directive. Add an optional checked
millisecond bound, immediate return when ordinary work is drained, typed timeout
with nonzero CLI exit, and no starvation inside the drain itself. `wait MS` remains
an intentional delay. Define settling around outstanding owned work, not the
lifetime of a language server or an endlessly following view.

Centralize the directive vocabulary used by parsing/help/error reporting. Document
`buffer`, `keys`, `key`, `paste`, `resize`, `frame`, `state`, `settle`, `wait` and
`quit-intent`, including quoting and key-token syntax. Do not advertise `open` or
`type` as directives unless implemented; show the supported `keys :e ...<cr>` and
insert/paste forms. Unknown directives name the valid vocabulary and help entry.
Update both usage lines to advertise file locations and remote directories.

Headless startup currently leaves `Editor.state_dir` unset. Separate trust access
from automatic session restoration/writes: share state-root resolution with the
TUI, honor persisted project trust, and make explicit `:trust` reachable under
headless operation. Tests use isolated HOME/XDG roots. If an explicit state-root
option is needed, it is a path override, not a blanket trust bypass. Never treat a
project's own config as consent, and never auto-approve project commands. Preserve
endpoint/root isolation and full replay without native trust/filesystem accesses.

### Release notes

Extract the matching version section from the checked-in CHANGELOG for GitHub
release creation. Missing/empty version notes are a release error, not a compare
link fallback. Keep the compare link as supplementary context. Backfill the
existing 0.16.0 and 0.17.0 release bodies from their corresponding sections without
changing tags, binaries, checksums or attestations. Document headless usage in the
README/site alongside the new remote flow.

## 5. GUI research deliverable, not implementation

**Follow-on contract:** [0061](0061-gui-windows-and-wsl.md) now owns the complete
GUI arc and evidence ledger; [0062](0062-distribution-and-wsl-onboarding.md) owns
distribution. The user chose WSL execution/storage as the initial envelope:
native Windows GPUI presentation over the existing Linux engine. The framework
comparison here is historical context, not permission to omit full TUI parity.

[0056](0056-architecture-prerequisites.md) closes shared engine, recovery,
execution/lifecycle and non-graphical protocol/server gaps. The separate
[0057](0057-core-verification-and-assurance.md) release immediately qualifies the
pre-worker core; [0058](0058-unified-native-worker.md) then unifies/requalifies native
services before 0059 completion and 0060 debugger. 0061 consumes these foundations,
not generic repairs, worker deployment or baseline verification left for GUI.

**Verdict: keep the TUI first-class, and add an optional native GUI later over the
same engine. Do not replace the terminal editor and do not build the GUI now.**
The Emacs-shaped choice is compelling: terminal access stays lightweight and
ubiquitous; a GUI can add reliable IME/composition, controlled fonts/HiDPI, direct
clipboard integration and an accessibility tree. A second set of editing semantics
would be a regression, not a benefit.

**Preferred first prototype: GPUI + gpui_platform + AccessKit**, with our existing
grammar/documents/jobs below a frontend boundary. GPUI's custom Elements explicitly
support efficient code-editor layout; its platform layer supplies Metal on macOS,
Wayland/X11 on Linux and Win32/DirectWrite on Windows. Its crate is Apache-2.0; this
is not a proposal to copy or depend on Zed's editor implementation. Pin versions:
GPUI is pre-1.0, warns about breaking changes and remains coupled to Zed's evolution.

**Windows is a first-class, native acceptance target**, not a best-effort port and
not something WSL testing proves. GPU acceleration and a polished minimal visual
design faithful to today's strop (monospace text, restrained chrome, strong typography,
current palette) are requirements. [Zed's Windows documentation](https://zed.dev/docs/windows)
states DirectX 11-compatible GPU support; the native GPUI Windows crate includes
AccessKit's Windows adapter. This strengthens GPUI as the first prototype choice,
but is not evidence that our own custom editor widget already works on Windows.

The Windows slice must cover Intel/AMD/NVIDIA and hybrid-GPU laptops, 100/125/150/200%
scaling and mixed-DPI monitors, Japanese/Chinese IME preedit and candidate placement,
DirectWrite/font fallback, Narrator/NVDA text and selection access, clipboard,
keyboard layouts, and a usable signed/packaged application. No Vulkan-only assumption,
software-only renderer, or webview approximation is accepted merely to claim support.

The current GUI plan keeps Unix filesystem/process/service ownership in WSL and
adds a bounded Windows presentation/input bridge. A native Windows workspace,
Windows LSP/DAP process-tree port and ConPTY are not prerequisites for that
envelope. They remain later backend capabilities. The native frontend still
requires Windows GPU/IME/accessibility and real bridge/lifecycle evidence; WSLg
alone is not that proof. Generic engine-boundary gaps are owned and closed by 0056.

| Candidate | Fit and concrete reservation |
|---|---|
| [GPUI](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md) | Preferred for an editor-specific native rendering/input surface; API churn and platform integration are real maintenance costs |
| [egui/eframe](https://github.com/emilk/egui) | Permissive, portable, custom drawing and AccessKit; excellent for tools, but its own docs warn of API churn and native-looking UI is a non-goal; complex editor text/IME still needs a custom surface |
| [Iced](https://github.com/iced-rs/iced) | MIT, message/update/view model fits our actions, GPU and software renderers; remains experimental and a large-text editor/accessibility surface still requires explicit integration |
| [Slint](https://github.com/slint-ui/slint#license) | Stable 1.x declarative tooling, multiple renderers; introduces another UI language and a licensing choice (GPLv3/royalty-free/commercial), not a drop-in permissive editor layer |
| Webview | Mature browser text/accessibility facilities, but adds a browser/platform runtime and another language/IPC boundary; not the default direction for this small Rust editor |

GPUI's [accessibility documentation](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/_accessibility.rs)
is explicit that custom elements must expose stable identities, text runs, selection
and accessible actions. Merely depending on AccessKit is not accessibility proof.
Likewise, reading a toolkit's README is not IME or platform verification.

The 0061 prototype gate is one real native Windows + WSL slice: shared edit/undo/
search, composition preedit/commit, Unicode/fallback, remote navigation, accessible
text/selection, HiDPI, large-document latency and actual automation/capture.
Other native GUI platforms have later explicit gates. Actions replay against the
same engine; pixel/shaping layout never replaces byte-domain grammar or terminal
cell coordinates. The TUI keeps its static/no-GUI-dependency and rustls-only rules.

If GPUI fails accessibility, IME or packaging gates, evaluate Iced/egui using the
same slice rather than accepting those regressions. The recommendation is a P3
prototype direction, not a framework commitment or a near-term delivery promise.

## 6. Implementation order and acceptance

1. Publish this plan; complete the indentation/GUI source research alongside the
   concrete UI/latency design notes, before their respective implementation.
2. Remove the demonstrated picker stall and audit/fix the other interactive paths.
3. Implement remote chooser and metadata-backed directory presentation on the same
   bounded worker/publication machinery.
4. Repair Python bootstrap, headless trust/settling/help and release-note publication.
5. Implement structural guides without reintroducing render-time unbounded work.
6. Exercise real terminal flows and TestBackend grids at wide/narrow sizes. Include
   remote failure/cancel/return navigation, unknown attributes, hostile/native names,
   Python available only by versioned/nonstandard path, headless trust and timeout,
   large search streams, stale worker completion and full native-result replay.
7. Run `docker compose run --build --rm test` and the applicable complete model gate;
   update existing docs/changelog/site and the priority-labelled roadmap. No ignored
   failing test, formatter/linter suppression or unverified partial implementation.
8. Commit and pass hosted CI, then publish 0.18.0 through the release workflow.
   Verify the platform artifacts, meaningful GitHub release body, demo and deployed
   website. Backfilled 0.16.0/0.17.0 notes must not alter their released artifacts.

Remaining optional scope stays explicit in 0035/0028: writable directory operations
after RW4, arbitrary remote shells/build/debugger, additional transports/Dev
Containers, and GUI implementation after native Windows/IME/accessibility proof.
The user has now requested remote-file editing as the next separately planned,
verified release after 0.18.0. Its conflict/atomicity/metadata/symlink gate remains
load-bearing. None of the current release's named fixes is silently deferred.

## 7. Implementation ownership and measured evidence

| Path | Interactive-side work | Owned preparation/publication |
|---|---|---|
| Local/remote file pickers | Query/caret/selection updates and immutable catalog handles | Persistent nucleo ranking actor; source batches use backpressure; picker/query tickets reject superseded results |
| Event delivery | Priority input lane; at most 32 background events or 2 ms between draws | Native senders wake the driver; headless uses the same event reducers |
| Grammar and pending previews | Small bounded pure reads; accepted command/typeahead queue | Frozen-rope resolver owner; the same result supplies preview, execution, repeat and macros |
| Syntax, injections, guides, search counts | Revision/viewport lookups and visible spans | One analysis owner keeps parsers and indexes, consumes edit journals and cooperatively cancels obsolete requests |
| Long-line geometry | Sparse checkpoint lookup and bounded suffix traversal | Worker-built indexes installed only at the owning buffer revision |
| Git surfaces/gutters | Indexed signs, row lookups, native file selection, visible tree rows | `HunkSet`, `PreparedDiff` and `PreparedFiles` are immutable shared worker results; hunk previews retain their document/revision ticket |
| LSP request text | Owned rope-slice capture, request identity and metadata admission | UTF-16 conversion and wire serialization run on the wire owner; filesystem/server discovery remains owned work |
| Files, remote directories, clipboard and shell | Intent registration and owner/freshness checks | Existing I/O/process jobs; directory metadata/text is prepared before publication |
| Native shutdown | Cancellation/retirement requests, no parser join on input | Parser/ranking owners dispose their native state and post terminal events |

Measurements on the same debug binary/workstation, through the real headless
editor and its production event/render path:

- 100,000 on-disk files: all entries arrived. Twenty query/navigation/cancel inputs
  had input-processing-to-next-render p50 **2.170 ms**, p95 **2.579 ms**, max
  **3.078 ms**. Across 130 frames, render p95 was **2.053 ms**.
- A 1 MiB line, end navigation and typing: 12 frames, render p50 **1.742 ms**,
  p95 **1.897 ms**, max **1.959 ms**. The earlier unindexed debug render reached
  roughly 440 ms on this fixture.
- A real Git commit replacing 3,000 lines, then switching files: 23 frames,
  render p95 **2.717 ms**, max **4.066 ms**. The real buffer showed both the large
  delta and the second file's own stats/content.

These are measured UI-work latencies, not disk/network completion times or a
portable constant-time promise for arbitrary requested edits. Full replay also
passed for large-buffer `ciw`, dot-repeat, counted macros and search direction,
and separately for Git dive/projection publication. Oversized forensic values
remain explicitly refused under the existing trace cap; the large Git performance
run used metadata recording, not a falsely advertised complete forensic capture.

Actual terminal interaction covered Markdown rendering and the host chooser through
successful SSH directory opening. Real SSH/headless runs covered root/home navigation,
metadata rows and narrow split Markdown editing/undo; CMake used the real renderer.
The website's language/headless/remote sections were inspected in Chromium.
Both local compose gates passed: `test` (formatting, strict Clippy and workspace
tests with SSH required) and `model` (bounded checks, progress/witness cases and
deliberate fault variants). Hosted CI and release-artifact checks remain publication
gates, not assumptions inferred from these local results.

The first hosted run passed Rust but caught a changed checksum at the moving
TLA+ `v1.8.0` prerelease URL. The build now pins the
[stable `v1.7.4` asset](https://github.com/tlaplus/tlaplus/releases/tag/v1.7.4):
published SHA-1 `bee4a54f3ee3d4afc347c3240ec2d9e93b075104` matched the download,
and SHA-256 `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`
is enforced by Docker. The full model gate passed again with stable TLC 2.19.
Its aggregate temporal diagnostic is attributed only after checking that the fault
configuration contains exactly the sole expected final `PROPERTY`; fault runs must
also exit nonzero. No checksum or model-failure check was disabled.
