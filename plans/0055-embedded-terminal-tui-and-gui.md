# 0055 — Embedded terminal: TUI first, shared GUI integration later

Status: requested research and implementation handoff; **not scheduled or
implemented**. No emulator dependency has been added and no integration benchmark
has been run. The user explicitly wants the TUI as the first delivery milestone,
then GUI integration when the GUI itself is tackled.

This is the canonical terminal plan, extracted from
[0054 §12](0054-unified-filesystem-workspace.md). It does not block filesystem
operations, search polish, completion or the current whole-editor release. The
GUI remains separately gated by [0038](0038-remote-experience-and-responsiveness.md)
and [0028](0028-roadmap-and-review.md).

## 1. Recommendation

**Reuse an emulator, own the editor integration. Do not build a VT emulator, embed
a whole terminal application, or import Zed's terminal widget.** Separate three
responsibilities: terminal emulation/input protocols, PTY/process lifetime, and
frontend rendering.

My starting choice for Strop is **the published `alacritty_terminal` crate**,
with a small Strop-owned session boundary and renderer. It is Rust, Apache-2.0,
GUI-independent and already used by Zed, although Zed uses a pinned fork. Start
with upstream, not that fork, unless a specific required patch is demonstrated.

For a separate PTY adapter, **`portable-pty` is the first candidate**. Alacritty
also supplies PTY/event-loop facilities: evaluate whether they already meet our
ownership requirements before adding a redundant second PTY implementation.
Neither choice removes Strop's responsibility for cancellation, descendants,
bounded queues, private capture and nonblocking UI publication.

**`libghostty-vt` is the strongest challenger, not a bad original suggestion.** Its
embedding surface includes keyboard/mouse/paste encoding as well as VT state and
incremental rendering. That can remove substantial application-side work. Its
current unstable C API, Rust FFI boundary, Zig build requirement and source-package
integration must be weighed against that benefit. Recheck it when work starts;
this is intentionally not a permanent commitment to today's maturity snapshot.

Do not turn “Rust preferred” into an excuse to maintain a large homemade keyboard
protocol implementation. If Alacritty's application-side encoding or Unicode/
projection gaps make that necessary, a packageable libghostty-vt integration may
be the better choice. The preflight gate below decides with running evidence.
Ship **one** selected emulator and one coherent session implementation, not a
runtime backend menu or parallel implementations maintained forever.

## 2. What the available components actually expose

These are primary-source/API findings, not measured latency, binary size or proof
that a dependency already works inside Strop. Versions are a research snapshot;
pin and recheck the chosen release/revision before implementation.

| Component | Actual reusable boundary | Assessment for Strop |
| --- | --- | --- |
| `alacritty_terminal` | Published Rust library; inspected docs are 0.26.0, upstream master 0.26.1-dev; Apache-2.0. VT state, cells/grid, scrollback, alternate screen, events, damage, PTY/event-loop modules. No GUI renderer. | First emulator candidate: good Rust and release-packaging fit. Keyboard/mouse encoding is not a complete turnkey public facility in this crate. |
| Zed `terminal` / `terminal_view` | Workspace-internal crates with inherited `publish = false`; GPL-3.0-or-later; coupled to GPUI and Zed settings/theme/task/editor/workspace components. The terminal model consumes Alacritty. | Architectural reference, not a standalone dependency. Do not vendor GPL implementation into the MIT project without a separate licensing decision; do not import its GUI dependency tree for the TUI. |
| `portable-pty` | Published 0.9.0, MIT. PTY/ConPTY creation, child spawn, blocking reader/writer, resize, wait/kill handles. No emulator or renderer. | Useful independent transport component. Blocking operations stay on workers; `kill()` is not a complete descendant-supervision policy. |
| `wezterm-term` | MIT, GUI-free and PTY-free full emulator; screen/scrollback, keyboard/mouse encoding, alerts and changed-row APIs. Its core is not published as a stable public crates.io package; maintainer documents pinned-Git use without API stability guarantees. | Technically strong, but not the default for this project: workspace/API coupling and publication costs. Do not describe every WezTerm crate as unpublished—portable-pty and termwiz are published. |
| `libghostty-vt` | MIT project; C/Zig embedding API with VT state, scrollback/reflow, render state, key/mouse/focus/paste encoding and host-effect callbacks. No automatic Strop PTY/session/UI ownership. | Strong second prototype. Current docs explicitly warn of API instability; build integration requires Zig even through its CMake wrapper. |
| `libvterm` | MIT, C99, toolkit-independent VT220/xterm library with callbacks; Neovim's emulator foundation. | Mature alternative if a C boundary is acceptable. Investigate actual needed protocol/Unicode and integration behavior rather than choosing solely because Neovim uses it. |
| `vte`, `termwiz`, `vt100`, `tui-term` | Different layers, not interchangeable: vte/termwiz offer parsing/terminal building blocks; vt100 does maintain a screen model; tui-term is a Ratatui widget currently backed by vt100. | Useful references/probe tools. A parser alone is not an emulator. Do not select the long-term core merely to obtain a ready-made TUI widget that dictates the later GUI model. |

### Alacritty: useful APIs and the important missing piece

The public `Term` API includes `renderable_content`, `grid`, `resize`, `mode`,
`damage` and `reset_damage`; **it does provide damage information**. The embedding
application still owns translating that into its renderer and coordinating
snapshot/damage consumption. `EventListener` provides the event/effect bridge.
Its optional application-level vi navigation is not Strop's grammar: do not adopt
a second, subtly different Vim implementation for terminal Normal mode.

Alacritty's upstream keyboard encoder lives in application code under
`alacritty/src/input/keyboard.rs`, using window-input types. Zed has its own
GPUI-oriented encoder. Tracking keyboard protocol modes in the core does not by
itself encode a Strop key correctly. Prefer a small adapter around permitted
upstream logic/a maintained encoder, with provenance/notices preserved, over a
new protocol dialect or copying Zed's GPL mapping table.

The supplied event loop uses shared terminal state and a `FairMutex`. That is an
integration option, not permission for Strop's renderer to block on a parser
lock. Use a worker snapshot boundary or demonstrably nonblocking access; a fair
lock can still block an input→render turn.

### WezTerm: complete engine, nontrivial distribution boundary

`Terminal::advance_bytes` feeds its emulator; screen APIs expose stable-row/
sequence information and changed rows; key_down/key_up/mouse_event encode input.
The caller supplies a writer. This is genuinely useful, not merely an ANSI parser.
Its inspected manifest also pulls the cell/parser/surface component family and
image-related dependencies. That is an observed dependency surface, **not** a
measured binary-size verdict or proof every dependency can be feature-disabled.

The maintainer's publication warning matters beyond API taste: Strop publishes to
crates.io. A production dependency on an unpublished Git-only crate does not fit
that release path just because a local Cargo build succeeds. It would need a
maintained publishable/vendor arrangement with licenses and upgrade ownership.
Do not casually propose a fork as zero-cost reuse.

### Ghostty: more off-the-shelf protocol code, explicit FFI obligations

Its C API can feed VT bytes, publish incremental render state and configure key
encoding from the active terminal modes. Host effects are explicit callbacks.
Those callbacks run synchronously during VT processing and must not re-enter the
same terminal or block on a clipboard/permission dialog. Queue bounded owned
messages or apply a pre-established deny/allow policy instead.

A CMake wrapper does not eliminate the Zig compiler requirement. Prove pinned,
static release builds and the crates.io/source-install story. Prebuilt Strop
users must not need Ghostty installed, a shared Ghostty library or Zig at runtime;
source packaging must not hide a network compiler download in build.rs. Keep
any unsafe FFI in a small audited adapter with named lifetime/thread invariants.

## 3. Evidence in the current Strop architecture

The inspected tree still has a finite shell-output feature, not an embedded
terminal:

- `editor/shell.rs` runs `:!cmd` as an owned job and displays its output when the
  job returns. `DocumentSource::Output` is named read-only text. Keep this useful
  finite-command path; do not force every task into a PTY.
- `crates/strop/src/terminal.rs::expand_key_event` normalizes legacy Alt to Esc +
  base key, filters release events and converts Ctrl-C to `QuitIntent` **before
  the engine knows the input target**. `key_from_event` maps only a subset of
  keys. These are intentional editor semantics from 0048, but insufficient for
  an embedded child application.
- `AppEvent::Terminal(Key)` currently means outer-terminal input translated to
  editor keys, not output from an embedded terminal. Clarify that naming during
  the cutover; don't create two unrelated meanings of “Terminal event.”
- `strop-core::process::OwnedProcess` owns a private Unix process group and may
  wait in Drop. It explicitly refuses non-Unix supervision. It is a pattern to
  reuse, not an already-portable PTY/session supervisor.
- `async_pending`/shutdown drains and headless `settle` currently concern finite
  work. A live terminal is a long-lived service; adding it as “pending until the
  shell exits” would hang normal settle/shutdown behavior.
- The terminal frontend owns alternate-screen/keyboard-mode restoration. Embedded
  output must never bypass that owner and write escape sequences to stdout.

No source changes are authorized by this handoff itself. These are integration
anchors; 0051/0053/0054 may split or move them before terminal implementation.

## 4. Preflight gate: choose with a real prototype

This is a prerequisite to the first milestone, not a third product milestone and
not evidence already completed by this research.

Build two disposable candidates outside production source: **published Alacritty**
and **a pinned libghostty-vt revision**. Use the same input/output corpus and actual
PTY application scenarios. WezTerm/libvterm are fallback candidates if either
reveals a decisive gap; do not implement four competing full integrations.

The comparison must answer:

1. Can it run an actual shell, pager and nested modal editor, including Esc,
   Ctrl-C, Ctrl-R, Alt, function keys, application-cursor mode, bracketed paste,
   alternate screen, cursor queries, resize/reflow and CJK/combining/emoji text?
2. Can cells, cursor, scrollback and text-navigation projection be exposed at
   bounded cost without a UI-thread lock, unbounded copy or inconsistent snapshot?
3. How much key/paste/mouse protocol code remains ours? Are host effects safely
   intercepted? Does the chosen input protocol reflect what the outer frontend
   can actually preserve?
4. What is the real PTY/session cleanup behavior with foreground jobs, background
   descendants, parent exit before final output, cancellation and multiple views?
5. Does it pass the project's static/dependency/license gate and crates.io package
   verification? Record binary/build/RSS/latency measurements; don't infer them
   from implementation language or a terminal application's marketing benchmarks.
6. What breaks or is unsupported? Record actual cell/input failures and maintenance
   costs. A no-go verdict is valid. Updating this plan's provisional choice needs
   evidence, not an allegiance to one emulator brand.

Record versions/revisions, toolchains, platforms, launch commands, captures and
observed limitations in the implementation ledger. Select one core before broad
integration; remove the losing throwaway prototype. Do not ship a mock terminal
or quietly narrow the interaction contract to make the preferred core pass.

## 5. Delivery milestones and required ledgers

### Milestone 1 — usable embedded terminal in the TUI

Scope: real **local** interactive terminals on the supported TUI platforms, with
Strop-owned lifecycle, keyboard input, terminal Normal mode and bounded rendering.
Existing Unix/WSL support is not proof of native Windows support. If Windows TUI
is supported when this milestone starts, native ConPTY evidence is required too.
Otherwise its port remains an explicit platform prerequisite, required before
claiming Windows GUI terminal support in milestone 2.

| ID | Required TUI result |
| --- | --- |
| T01 | Real PTY-backed shell/explicit-command launch in a captured local context, with truthful starting/running/exited/failed state. No pipe-only fake fallback. |
| T02 | Complete supported keyboard routing before lossy editor normalization; terminal-input mode and explicit escape to editor Normal mode. |
| T03 | Terminal document with bounded live screen/scrollback, normal Vim navigation/search/selection/yank and a coherent read-only text projection. |
| T04 | Pane-clipped Ratatui rendering, source-independent terminal colors/cursor, alternate-screen and resize/reflow behavior, including narrow/tiny recovery. |
| T05 | Owned PTY/session lifetime, EOF/exit draining, termination and descendant policy; closing a view is not silently killing or orphaning its process. |
| T06 | Bounded reader/parser/writer/frame queues and memory; no input→render await, blocking parser lock or lossy VT/input stream coalescing. |
| T07 | Safe host-effect policy, accurate capability/environment reporting, scoped cwd/title metadata and clipboard/paste boundaries. |
| T08 | Private capture/replay/session rules; no password-bearing input by default, no automatic command execution on restore/replay. |
| T09 | Multiple terminal buffers and ordinary editor splits/returns; one resize authority per shared session; explicit filesystem “terminal here” integration where supported. |
| T10 | Real interactive/physical-terminal evidence, meaningful regression corpus, command help/docs/changelog, package/static checks and the integrated repository gate. |

T01–T10 are all required to call the TUI milestone complete. A colored shell-log
buffer is not a terminal; a VT grid demo without process/input/lifetime integration
is not the release. An omission needs explicit user approval recorded in 0028.

The first TUI contract is keyboard-first. General pointer interaction remains
under its existing separate policy; if terminal mouse forwarding is enabled, it
must satisfy §8's scoped routing contract. Do not advertise unsupported input or
swallow unrelated editor/outer-terminal mouse interaction.

### Milestone 2 — GUI surface over the same terminal core

Starts after milestone 1 and after the GUI's own platform/engine integration gate.
**Reuse the terminal session, emulator, PTY, input/effect policy and replay model.**
Add a frontend, not a second terminal implementation or a GPU application embedded
inside the GUI.

| ID | Required GUI result |
| --- | --- |
| G01 | Shared terminal documents, process ownership and command policy; TUI remains first-class and independently buildable. |
| G02 | Native GPU cell rendering with correct clipping, font fallback, grapheme/cell alignment, cursor, selection and incremental redraw. |
| G03 | Native key/IME preedit/commit, clipboard, pointer/wheel and terminal-mouse ownership without duplicate input or editor shortcut theft. |
| G04 | Pixel→cell resize authority, mixed-DPI/font/zoom transitions and multi-view behavior without PTY resize feedback loops. |
| G05 | Accessible terminal text/selection/actions, bounded output announcements, focus and keyboard-only navigation—not just a linked accessibility crate. |
| G06 | Real supported-platform evidence, especially native Windows/ConPTY and Windows GUI→WSL context handling; WSL TUI evidence is not this gate. |
| G07 | Same security/private capture/replay and close/restart guarantees, with actual GUI walkthroughs and no TUI regression. |

GPUI is the currently preferred GUI evaluation direction from 0038, not a required
terminal dependency. If GUI selection changes, the terminal core does not change
with it. Zed's GPUI integration is a useful reference for responsibilities; its
terminal_view is not the component we import.

## 6. One session service, two renderers

```text
TUI native events now / GUI native events later
                    |
          input target + session admission
                    |
        bounded terminal intents (keys/paste/resize)
                    v
        terminal session worker / selected VT engine
          |              ^                  |
          v              |                  v
   ordered PTY writer   PTY reader     immutable frame/text data
          |              ^                  |
          +---- owned child/session --------+----> TUI / GUI views
                    |
           lifecycle and host effects
                    v
          editor-owned policy + event loop
```

A small `strop-terminal` crate is a reasonable ownership boundary: one real
emulator adapter, PTY/session supervision, input codec integration, snapshot data
and typed effects. Engine document/focus/command policy stays in strop-engine;
Ratatui/GPUI and font objects stay in their frontends. Reuse core worker primitives
without putting blocking child handles in editor state. Do not build a generic
terminal-backend plugin framework merely because the research compared libraries.

The model needs meaningful identities at real boundaries: session ID/incarnation,
view/input owner, frame/geometry revision, launch context and process capability.
A PID, a window title or a `term://` display name is not a session identity or
permission to launch a command.

Worker ownership must be explicit:

- One logical owner serializes emulator mutations, resize and mode-dependent key
  encoding. UI code never concurrently pokes the live grid.
- Native PTY read/write/wait/Drop run off the UI thread. They may be blocking
  operations on owned workers; asynchronous APIs alone do not prove responsiveness.
- Output parsing yields prepared immutable data. The UI installs bounded updates
  and paints only its visible area. Reuse unchanged row data; do not stringify
  the entire scrollback or copy every cell on each key/frame.
- Protocol responses and user input use one ordered writer. Handle partial writes;
  never interleave a response into a bracketed-paste envelope or declare bytes
  executed merely because they entered a queue.
- Coalesce **render notifications/snapshots**, not VT bytes or input. VT is stateful:
  dropping an escape prefix can corrupt all subsequent output. If deltas are used,
  carry a base revision and recover from a missing base with a current snapshot.
- Apply explicit byte/count limits to output, input/paste, scrollback, effects and
  sessions. Backpressure the reader/child when necessary; never accumulate an
  unbounded event queue while still calling the UI “nonblocking.” A rejected paste
  or full input queue is visible, not a silent loss of user input.

## 7. Terminal document and TUI interaction

### Launch and placement

`:terminal` starts the configured/default interactive shell in a terminal buffer.
An explicit command form runs exactly the user-requested command. Use the normal
split/window machinery for placement; do not force a permanent bottom drawer or
invent terminal-only tabs. Existing `:!`/pipe behavior remains useful and unchanged.

Capture namespace, cwd, program/arguments and relevant environment at admission.
Default terminal launch follows a supported local context. In an SSH/container
context, refuse unsupported interactive execution instead of silently opening a
local shell against a remote-looking path. Provide an explicitly labelled **Open
local terminal** action for that deliberate choice; it is not a fallback hidden
inside `:terminal`.

“Terminal here” from 0054 supplies a typed local directory and launch intent. It
must not synthesize `cd <display path> && shell`. Environment changes belong on
the child command, not process-global `set_var` calls after editor workers exist.
No terminal library's global environment setup is used blindly.

A real PTY/spawn success establishes an available transport, not a guessed shell
prompt-ready state. Do not screen-scrape `$`/`%` or insert arbitrary startup sleeps.
Display starting/failure honestly, and preserve input/output ordering during launch.
The delayed spawn result cannot steal focus back from a pane the user selected.

### Terminal-input versus terminal-Normal mode

- Terminal-input sends keys to the child: Esc, Ctrl-C, Ctrl-R, Ctrl-L and Ctrl-W
  keep their child/application meanings. Ctrl-C is normally a byte delivered to
  the PTY so termios/the foreground application decides what it means; it is not
  Strop QuitIntent or an unconditional kill of the shell.
- **Ctrl-\\ Ctrl-N** leaves terminal-input for Strop Normal mode. `i`/`a` return
  to child input and the live cursor. Esc alone must keep working inside nested
  Vim/Neovim and other modal applications.
- The escape prefix has an explicit owner. A nonmatching second key forwards the
  literal prefix plus that key in order; it must not disappear. Focus loss cancels
  the editor escape-prefix state without routing the next key into another session.
- In terminal-Normal mode, ordinary Strop motions, `/`/`?`, marks, selections and
  yanks operate on a coherent read-only terminal text view. Do not use Alacritty's
  separate vi-motion implementation as a shortcut. Text-changing operators do
  not delete terminal history, send commands or undo child side effects.
- Output continues while inspecting scrollback without dragging the view to the
  bottom. Returning to terminal-input explicitly follows the live cursor again.
  Exited terminal buffers remain readable; restarting is an explicit new session.

### Screen text is a projection, not an ordinary editable file

The VT engine owns screen cells, soft wraps, cursor and scrollback. The editor
projection must preserve that truth while making text navigation useful. Hard
newlines and soft wraps are not the same thing. Combining/wide characters occupy
terminal cells differently from byte positions; use explicit mappings.

Use stable logical line anchors where the selected engine supports them. When
reflow/trimming or live cursor edits make a selected inspection range impossible
to preserve, retain an immutable inspection snapshot or invalidate it visibly—
never yank bytes from a different row under an old highlight. Cells, text and
selection must belong to the same snapshot/generation. Scrolling history is not
an excuse to rebuild the whole history on every output chunk.

A terminal has no writable filesystem binding. Export/copy produces an explicitly
named text snapshot; `:w` cannot save VT state to a guessed file or send text to the
child. A display URI is inert and must not be reparsed as a shell-launch recipe.

## 8. Input, geometry and rendering boundaries

### Preserve input before applying editor grammar policy

Move the lossy decision out of the outer reader's unconditional path. Frontends
emit a frontend-neutral event that preserves supported key code, modifiers,
press/repeat/release information and text/paste. The ordered engine admission
selects **editor input** or **terminal input** before normalization:

- Editor path retains 0048's Esc/Alt preservation, Ctrl-C policy and normal Vim
  semantics. Its regressions stay green.
- Terminal path keeps the original supported facts and encodes against the child
  terminal's active modes. Do not reconstruct Alt/function/control keys from the
  reduced Editor::Key enum after information has already been discarded.
- Legacy terminals cannot distinguish every Esc-prefix/Alt/control combination.
  State that limit. Negotiate richer keyboard modes only when the outer frontend
  can preserve and forward the required events; do not fabricate key releases or
  advertise an input protocol that the chain cannot deliver.
- Native/GUI IME text commit is distinct from physical keys. Preedit is not sent
  to the child; commit is delivered once, without a duplicate key-text path.
- Bracketed paste is one bounded explicit intent, encoded according to child mode.
  Do not turn it into editor keystrokes or silently append Enter. Control/multiline
  paste follows an explicit safety policy, including embedded delimiter handling.

Input ownership is decided in the same ordered event stream as focus/mode changes,
not from an unsynchronized reader-thread snapshot of “which pane was active.”

### One PTY geometry, potentially several views

A session has one screen size and one controlling view. Two differently sized
views of the same terminal cannot each resize its PTY on every paint. The focused
controlling view establishes the desired cell geometry; other views mirror/clip
or navigate that same state. Coalesce resize intents and version their outcomes.

Use the **inner pane** dimensions after chrome, not the outer terminal's full
size. Do not resize a hidden/tiny pane to an invalid zero grid. Resizing both the
emulator and PTY belongs to the owned session; stale frames from old geometry
cannot corrupt another pane. Pixel sizes are unknown in many TUI contexts—do not
invent them. GUI font/DPI changes later supply real cell metrics.

TUI rendering consumes cells, colors/styles, cursor and selection, clipped to the
pane. Child erase-screen, alternate-screen and cursor sequences affect only that
virtual terminal. The outer alternate screen and cursor visibility/style remain
under Strop's frontend owner. Do not emit the child's raw VT bytes or an emulator's
ANSI “formatted screen” output straight to stdout.

Use existing visual language for the surrounding chrome: trusted local/namespace
identity, shell/process state and mode are distinct from an untrusted child title.
Terminal content colors are terminal colors, not syntax-highlighting guesses.
CJK, combining text, emoji/ZWJ, continuation cells and clipped wide characters
need actual outer-terminal evidence; missing font glyphs and grid-width bugs are
different failures. Fix a model mismatch rather than adding arbitrary padding.

If terminal mouse reporting is added, translate coordinates relative to the
controlling pane and only send events owned by that terminal/input mode. Balance
outer mouse capture once, preserve editor/outer-terminal interaction elsewhere,
and distinguish selection/scrollback gestures from child mouse input. No global
unconditional mouse forwarding as a shortcut. The GUI milestone requires this
ownership distinction for its native pointer integration.

## 9. Process lifetime, shutdown and service liveness

Track session state separately from visible buffers: starting, running, closing,
exited(status) or failed(error), plus retained output/inspection state. An exited
shell is not permission to drop unread final PTY bytes. Drain within an explicit
policy; account for descendants that still hold terminal handles open.

Closing a view, hiding a terminal buffer, terminating a session and exiting Strop
are different actions. Hidden live sessions remain owned and discoverable.
Destructive close/quit with live work requires a deliberate policy/confirmation;
never silently orphan it or throw away the result. Keep the final transcript and
exit status readable. Reopening an exited terminal view does not relaunch it.

`portable-pty` supplies handles, not the full supervisor. Its inspected Unix
`Child::kill` targets the direct child; do not equate that with killing every job.
Interactive shells create foreground/background process groups. Likewise, taking
Strop's existing single process-group kill and assuming it covers every PTY job
is unsafe. Design controlling-terminal/session ownership, cancellation and reaping
for each platform; use appropriate Windows job/process ownership when ported.

Test shell, foreground child and background child closure explicitly. State the
limit for deliberately daemonized/detached processes rather than promising a
universal kill tree without OS support. Never signal a reused/unverified PID; a
revoked process capability cannot later become a kill of an unrelated process.
Native handle Drop/wait and grace-period escalation stay off the UI thread.

A live terminal is a service, **not an eternally pending finite job**. Headless
settle waits for specified admitted work/output/frames, not for the interactive
shell to exit. Shutdown explicitly requests terminal closure and drains outcomes
under its policy; no hang caused by blindly extending async_pending. Emulator,
reader or writer failure must terminate/reconcile ownership and produce a visible
error, not leave an undriven child or a permanently spinning buffer.

## 10. Security, namespace and private capture

Treat output as terminal data, not instructions to the editor:

- VT replies such as cursor/size reports go to the originating child and reflect
  its virtual pane. They do not leak unrelated editor buffer or host-window state.
- Title/bell metadata is bounded and sanitized. A child title cannot overwrite the
  trusted namespace/principal label, rename a document or alter global UI policy.
- OSC cwd reports are scoped, validated metadata. Distinguish **launch cwd** from
  **shell-reported cwd**; absent shell integration, we do not know subsequent cd
  commands. Never change process-global/editor cwd from terminal output.
- Clipboard reads are denied by default; clipboard writes also require explicit
  scoped policy. Do not open a blocking permission dialog in an emulator callback.
  Manual user selection/yank is a different, explicit authority.
- Hyperlinks and path:line candidates never auto-open or execute. Explicit source
  navigation uses captured/validated location context; a local child's OSC URI
  cannot silently acquire a remote connection or arbitrary execution authority.
- Window resize/control, downloads, notifications and unknown host-effect sequences
  are bounded and denied/handled by policy, never passed raw to the outer terminal.
- Advertise only capabilities the selected emulator **and frontend** can provide.
  No fake Kitty/sixel graphics support just because a parser recognizes the escape
  sequence. No application-name/TERM claim that silently requires an unavailable
  terminfo entry; select a truthful portable profile and test it.

Use explicit user launch authority. Project files/session restore cannot start a
shell automatically. Remote interactive terminals, SSH forwarding/elevation and
container terminals need separate capabilities; a fixed trusted filesystem helper
or remote-save permit is not a general interactive-shell grant. An unsupported
remote launch cannot fall back to the local filesystem/process namespace.

### Terminal data is especially sensitive

Passwords may be entered while echo is disabled, but that is not a reliable
secret detector. **Do not record terminal input, paste, full command/environment
or PTY content by default.** Apply the rule before generic event recording, not
after the raw key has already reached the tape. Cover buffer/frame capture too:
redacting input events does not help if a terminal cell-grid export leaks output.

Provide a deliberately enabled, private terminal-content capture mode for bug
reports/replay, with clear scope and visible indication. Metadata-only traces
must say that terminal-content replay is unavailable, not pretend an empty screen
is an equivalent replay. Full capture records enough input/output/resize/effect
ordering and the engine/version context to replay without any native process,
clipboard, network or filesystem execution. Persisted sessions may restore an
inert placeholder/snapshot; reconnect/relaunch requires fresh explicit intent.

## 11. GUI integration after the TUI milestone

The GUI consumes the same session/frame/text/effect contract, with native event
translation and rendering. Do not import an Alacritty/WezTerm/Ghostty application
window or Zed terminal_view as a nested application. Do not replace the TUI model
with a GUI-owned terminal whose state cannot be replayed/headlessly exercised.

Concrete GUI work:

- Paint the VT grid using the chosen GUI renderer and shared palette data.
  Fonts/fallback/shaping must respect terminal cell allocation, continuation cells
  and cursor positions; ordinary paragraph layout is not a terminal renderer.
- Maintain a precise pixel→cell transform for pointer, cursor/selection, IME anchor
  and resize. Mixed DPI, zoom and font changes update one geometry authority.
- Route native keys and IME correctly into the same input intents; preedit stays
  in the GUI until commit. Context menus/actions do not inject shell text as a
  substitute for typed terminal actions.
- Implement clipboard and child-mouse modes through the existing security/input
  owners. Don't bypass them with a convenient GUI widget callback.
- Expose terminal text, selection and actions through accessibility with bounded
  output announcements. Test actual screen-reader behavior and focus transitions;
  a generic canvas or AccessKit dependency is not accessible terminal content.
- Preserve lifecycle when panels/tabs/windows hide or close. Rendering resources
  are disposable; process/session state is not owned by a GPUI Element destructor.
- Validate native Windows ConPTY/process supervision and the already-planned
  Windows GUI→WSL boundary. OS-handle support in a dependency is not an implemented
  Strop platform port. Keep Linux/macOS/TUI gates independent and green.

Do not promise live transfer of a running process between separate TUI and GUI
instances. Sharing the implementation is required; cross-process handoff would
need a separate daemon/session transport and authority design.

## 12. Implementation map and validation

### Order and ownership

1. Run preflight, record the chosen emulator/PTy/encoding/package verdict.
2. Add the frontend-neutral terminal service/model and native supervision seam.
3. Cut input admission over without regressing existing editor normalization.
4. Integrate terminal documents, text projection, lifecycle and the TUI renderer.
5. Complete policy, private capture/replay, multi-view/resize and failure paths.
6. Exercise all T requirements, package/static gate, documentation and cleanup.
7. When GUI work resumes, implement G01–G07 over that same core and re-run the TUI
   corpus as well as actual GUI/platform evidence.

One integration owner owns the session/input/privacy contract. Backend/PTy,
projection/rendering and policy/test slices can run independently only after those
shared types and lifetimes are explicit. No per-frontend emulator, per-module
input encoder or duplicate process supervisor. Source modules stay within the
repository's size/complexity discipline; split by protocol, transport, model,
projection, effects and frontend concerns rather than one huge terminal.rs.

Use LSP references for exported-symbol changes and migrate old event names,
recording formats, help/callers/tests as one clean cutover. Unsupported historical
trace versions need an explicit diagnostic, not a misleading compatibility alias.
Existing shell-output and protected remote-operation paths remain separate.

### Behavioral proof required for milestone 1

Use private fixtures/HOME/XDG configuration, not the user's interactive shell
config for deterministic coverage. Readiness comes from observable handshakes or
protocol state, not wall-clock sleeps or prompt-string guesses.

- A real shell accepts commands and displays results; a pager and nested
  `nvim --clean` work. Esc affects the nested editor, Ctrl-C interrupts the child
  operation, and Ctrl-\\ Ctrl-N returns to Strop without terminating either.
- Byte/input fixtures cover arrows in normal/application mode, Ctrl-R history,
  Ctrl-W, Alt, function keys, enhanced/legacy input, literal escape-prefix recovery,
  repeat/release where supported, UTF-8 and bounded bracketed paste.
- VT fixtures cover partial byte/escape/UTF-8 chunks, erase/scroll regions,
  alternate screen, colors/cursor/query replies, soft wrap/reflow, CJK, combining
  marks and wide/ZWJ edge cases. Assert visible cells/text/input effects, not source
  code or a mock's echoed arguments.
- Resize/split/hide/mirror operations use one PTY geometry; delayed old frames do
  not corrupt a pane. Inspecting scrollback while output arrives preserves the
  selected text or explicitly retains its snapshot.
- Sustained output and backpressured stdin leave editor input/render responsive,
  bounded in memory and free of dropped protocol bytes/secret tracing. Record
  measured results and limits rather than an unsupported “zero latency” claim.
- Child startup failure, exit before drain, retained descendant handles, cancel,
  hide versus terminate, final-window quit and worker failure all yield correct
  session/exit state without orphaning owned jobs or signaling a reused PID.
- OSC title/cwd/clipboard/window/download attempts cannot cross namespace or host
  policy. Default and metadata-only captures contain no sensitive terminal payload;
  deliberately enabled capture replays without native execution.
- Existing 0048 input preservation, normal-buffer grammar, picker/completion focus,
  finite shell jobs and headless settle/shutdown remain correct.

Actual TUI evidence must include representative outer terminals (the user's
Windows Terminal→WSL path included), wide/narrow/tiny panes, Unicode and at least
one nested full-screen application. Golden TestBackend fixtures alone cannot
prove the outer terminal's width/input/restoration behavior. A fake PTY proves
neither job control nor cleanup.

Run the repository's `docker compose run --build --rm test` gate after integrated
implementation: fmt, locked workspace/all-target clippy and locked tests. Also
verify the actual release dependency/static and crates.io package/install path;
a local Git-linked build is not publication proof. Keep only regressions guarding
plausible behavior/boundary failures; remove disposable prototype scaffolding.
Update relevant existing docs/changelog, key help and the T/G acceptance ledgers.

For milestone 2, add actual GUI screenshots/interactions, IME, pointer/selection,
clipboard, accessibility, mixed-DPI and supported native platform/process tests.
Do not substitute WSL, a mocked GPU renderer or dependency presence for evidence.

## 13. Explicit follow-ons, not hidden milestone reductions

- Remote interactive SSH terminals, container exec terminals and terminal service
  forwarding need new transport/authority contracts. Existing remote read/write
  helpers are not the missing terminal backend.
- Terminal graphics/image protocols, downloads, shell integration/command markers,
  automatic diagnostic linking, task recipes, session persistence/reattachment and
  cross-process TUI↔GUI session transfer are separate extensions.
- General TUI mouse/pointer UX remains separately gated. Optional terminal mouse
  forwarding must meet the scoped contract before enabling it; GUI pointer
  integration remains required in G03.
- Native Windows support is an actual platform implementation/evidence gate,
  not an assumption from choosing a cross-platform crate.

T01–T10 or G01–G07 cannot be moved into this list merely to make a release pass.
Record any proposed scope change, reason, impact, re-entry condition and explicit
user approval in 0028. A completed TUI milestone can stand on its own while the
GUI is deferred, exactly as requested; it is not permission to ship an incomplete
TUI integration as a “terminal foundation.”

## 14. Primary sources

- [Alacritty terminal crate manifest](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/Cargo.toml)
  and [published Term API](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/term/struct.Term.html):
  license/dependencies, public grid/renderable-content/damage/resize/mode APIs.
- [Alacritty application keyboard encoder](https://github.com/alacritty/alacritty/blob/master/alacritty/src/input/keyboard.rs):
  the application-side encoding boundary, not a turnkey core feature.
- [Zed terminal manifest](https://github.com/zed-industries/zed/blob/main/crates/terminal/Cargo.toml),
  [terminal_view](https://github.com/zed-industries/zed/blob/main/crates/terminal_view/Cargo.toml)
  and [workspace manifest](https://github.com/zed-industries/zed/blob/main/Cargo.toml):
  internal publication policy, GPL licensing, GPUI coupling and Alacritty fork.
- [portable-pty API](https://docs.rs/portable-pty/latest/portable_pty/) and
  [implementation](https://github.com/wezterm/wezterm/tree/main/pty/src):
  native PTY interfaces, blocking handles and actual child-kill behavior.
- [WezTerm core README](https://github.com/wezterm/wezterm/blob/main/term/README.md),
  [manifest](https://github.com/wezterm/wezterm/blob/main/term/Cargo.toml) and
  [maintainer explanation](https://github.com/wezterm/wezterm/discussions/5217):
  complete emulator versus termwiz, no GUI/PTY, unpublished API/revision contract.
- [Ghostty project](https://github.com/ghostty-org/ghostty),
  [libghostty-vt API](https://libghostty.tip.ghostty.org/),
  [terminal/effect API](https://libghostty.tip.ghostty.org/group__terminal.html) and
  [CMake build notes](https://github.com/ghostty-org/ghostty/blob/main/dist/cmake/README.md):
  embedding scope, instability warning, synchronous callback rules and Zig build.
- [libvterm](https://www.leonerd.org.uk/code/libvterm/),
  [license](https://github.com/neovim/libvterm/blob/master/LICENSE) and
  [Neovim terminal behavior](https://neovim.io/doc/user/terminal.html):
  mature C alternative and terminal-buffer/input precedent.
- [vt100](https://docs.rs/vt100/latest/vt100/) and
  [tui-term](https://docs.rs/tui-term/latest/tui_term/):
  screen model and ready-made Ratatui widget, not a complete Strop session design.
- [Cargo dependency/publication rules](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations):
  why an unpublished Git dependency is a release-design decision, not just a pin.
