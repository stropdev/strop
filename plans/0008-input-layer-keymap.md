# 0008 — The Input Layer as Data (keymap restructure)

> Why the keymap must become a table, what we borrow, and what we
> deliberately don't build on.

Status: stage 1 landed (0.5.0): leaf commands are data
(editor/registry.rs), dispatched from the table; coverage pinned by
keymap tests. Stage 2 (the trie absorbing pending/operator state and the
BINDINGS/LEAVES merge) remains planned.

---

## 1. The problem

Key dispatch today is a hand-rolled match tree across
`editor/{normal,visual,insert,picker,git_memory}.rs` with interleaved
pending-string parsing. It works, but it is the class of code where
state bugs breed (the `Ctrl-r`-eaten-by-the-Char-guard class, the
pending-vs-direct split, visual ops missing transactions). Three surfaces
already need the same data and can't have it: the which-key overlay
(static hint lists that drift), the keybinds popup (0003 §5.7), and
config rebinds (0005 §4). One table fixes all three and kills the bug
class.

## 2. Why not embed an existing editor

Considered and rejected, with reasons that are the project's identity:

- **Neovim via `--embed` RPC** (VSCodeVim lineage): perfect fidelity —
  and it kills the product. The preview is a dry-run of the same pure
  resolver that executes; with nvim as the backend, the resolver becomes
  an RPC query per keystroke — the async invariant (0001 §5.6) and
  input-to-echo-under-one-frame die. The static single binary dies (you
  now need an nvim process). The no-plugin-runtime identity dies. The
  grammar becomes someone else's bug reports.
- **helix-core/helix-view as libraries**: not consumable as a vim
  grammar engine (their grammar is Kakoune selection-first — the exact
  thing this project exists to not be), and deeply coupled to Helix's
  editor shell.
- **Kakoune/Zed cores**: same class of answer.

The wheel we *don't* reinvent is the vim grammar itself — that's the
product. The wheel we *do* steal is the input-layer architecture.

## 3. The design (helix-term/keymap.rs lineage)

Keymaps become data; dispatch walks a trie; pending state is a trie
position.

```rust
pub enum Binding {
    /// A leaf command with a stable identity (dot-repeat, tests, `?` popup).
    Command { name: &'static str, desc: &'static str },
    /// A prefix node with its own hint table (Space, g, m, ], Space g).
    Prefix { desc: &'static str },
    /// Parameterized prefixes: counts, registers, f<t>, /pat, "a.
    Dynamic(DynamicKind),
}

pub struct Keymap { pub mode: Mode, pub trie: Node }
```

- **One table per mode** (normal, visual, insert, picker, readonly-surface).
  Buffer-local maps (git surfaces) are a table overlay, not a hardcoded
  branch — that's the fugitive convention made structural.
- **Dynamic nodes are explicit**: a `Count` node absorbs digits, a
  `Register` node absorbs `"x`, a `Find` node absorbs `f<char>` — no more
  pending-string puns.
- **Operator-pending is a trie level**, not a string: after an operator
  the walker is inside the operator's subtree; every complete path is a
  `strop-grammar` `Command` — the preview hook stays exactly where it is
  (resolve is unchanged; the trie only decides *when* keys complete).
- **Which-key renders the current trie level.** The `?` keybinds popup
  renders the tables. Coverage test: trie leaves == table entries ==
  `?` rows, mechanically (0003 §5.7's contract finally literal).

## 4. What moves, what doesn't

- `strop-grammar` (parse/resolve/preview): **untouched.** The trie's
  leaves construct `grammar::Command`s; the resolver is the contract.
- Modes, picker, git surfaces, ex-line: keep their handlers; their *key
  dispatch* moves to tables.
- Transactions (undo units) attach to leaf commands, not match arms.

## 5. Migration order (one cutover, no dual dispatch)

1. `keymap.rs`: trie + normal-mode table covering every current binding.
2. `feed_normal` delegates to the walker; old match tree deleted.
3. Visual/insert/readonly/pending tables follow in the same change.
4. Which-key overlay reads trie levels; static hint lists deleted.
5. `Space ?` popup reads the tables; `?` stays search-backward (vim).

## 6. Fidelity guard

The differential harness (0001 §5.10) plus the existing 83 editor tests
are the safety net; the restructure is done when both pass unchanged and
the trie coverage test passes.
