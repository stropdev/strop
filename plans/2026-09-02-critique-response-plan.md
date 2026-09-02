# Critique response plan — 0.17.14 (execute evening 2026-09-02)

Source: external adversarial review (static, post-0.17.13). Everything
below was verified against the code before planning; claims marked
[VERIFIED] were confirmed by reading current main.

## My assessment of the critique (agree / push back / stale)

**Agree, and acting on it:**
- **Deno precedence** [VERIFIED — `frontend.rs:25-31`]: order is
  `GRIPSACK_DENO → PATH deno (≥2) → pinned download`. The comment
  justifying PATH-first ("a site deno must win") is redundant with
  `GRIPSACK_DENO`, which already exists as that escape hatch. PATH-first
  buys nothing except silent runtime skew between "identical" machines.
  Flipping is the right call. **The user flagged this as the important
  one.**
- **profile.sh rendered after the flip** [VERIFIED — `apply.rs` ~line
  134, the exact M3 window from round 1]: `write_manifest → flip →
  render_env_file?`. An I/O failure there reports apply-failed while
  the generation IS active. Honest fix: render before the flip.
- **Enable GitHub issues** — "issue creation is restricted" is the
  single highest-leverage one-click change. Verify and fix.
- **"no custom language — TypeScript" wording** — correct nerd-snipe
  bait; "no new language to learn — typed TypeScript API" is stronger
  and defensible.
- **Crash-recovery journal as P0** — agree with priority; it is the
  load-bearing promise ("safely manage my machine"). Evening scope =
  write the plan doc only (it's a design project, not an evening patch).

**Push back:**
- **Linter long-tail**: the architecture advice is right; "stop
  maintaining packs" is a product call with a real tradeoff — the packs
  are data-driven and cheap per-pack, and they're a visible moat. My
  recommendation: keep ~6-8 showcase packs first-party, move the rest
  to a contributed label, and stop auto-filing "tables lag upstream"
  issues for non-showcase packs. Present to Tarek; don't execute
  unilaterally.
- **Over-modularization risk**: mild disagree — 9 crates at ~36k loc
  with one dependency direction is not over-modularized; the critique
  itself says it'd take this over a monolith. No action.
- **Comparison table**: agree it should be smaller and more hostile to
  us; splitting chezmoi/Stow and dropping the "config linters with
  spans" row (a criterion invented to win) is right. Execute as
  website copy edits (Tarek approves the final table).

**Stale check:** nothing I verified was stale — apply.rs mechanics,
deno precedence, changelog regression history, CI gates, e2e suite
all read as described against post-0.17.13 main. Adoption/maturity
scores are opinions, not facts to fix.

---

## P0-A: Deno precedence flip

**File:** `crates/gripsack-exec/src/frontend.rs`

**New order:** `GRIPSACK_DENO` → pinned → PATH (≥2, LOUD) → error.

```rust
pub fn ensure_deno(home: &Path) -> io::Result<PathBuf> {
    // explicit override always wins (doctor reports it)
    if let Ok(deno) = std::env::var("GRIPSACK_DENO") {
        return Ok(PathBuf::from(deno));
    }
    // pinned by default — two "identical" machines must eval through
    // the same runtime; a PATH deno used to win here silently
    if let Some(deno) = pinned_deno(home)? {
        return Ok(deno);
    }
    // pinned unavailable (musl host, or download failed): a PATH deno
    // is the fallback, loudly — never a silent skew
    if let Some((deno, version)) = deno_on_path() {
        tracing::warn!(
            runtime = %version,
            "pinned deno unavailable — using deno from PATH; set GRIPSACK_DENO to make this deliberate"
        );
        return Ok(deno);
    }
    Err(io::Error::other(
        "no usable deno: pinned unavailable and none on PATH (see `grip doctor`)",
    ))
}
```

- `pinned_deno(home) -> io::Result<Option<PathBuf>>`: exists-check →
  `Some`; musl host → `Ok(None)` (no build ships — keep the existing
  named-error path for the no-fallback case in the caller's final
  error); else flock'd download (keep existing provision lock + race
  re-check); **on download failure**: if a PATH deno exists →
  `Ok(None)` after warn (availability preserved); else return the
  download error verbatim (it names the real cause).
- `deno_on_path() -> Option<(PathBuf, String)>` — also return the
  version string for the warning. Keep the ≥2 major gate.
- **Doctor** (`commands/doctor.rs`): the "(provisioned)" label logic
  must distinguish override / pinned / PATH-fallback and show the
  version in the PATH case.
- **Tests:** update `frontend.rs` unit tests (139-186) for the new
  order; PATH-deno test now expects fallback-not-preference. e2e is
  unaffected — [VERIFIED] the docker image sets `GRIPSACK_DENO` to
  the prefetched pinned binary (`Dockerfile:48`), never relying on
  PATH-wins.
- **Changelog:** behavior change with migration note (site denos →
  `GRIPSACK_DENO`).

## P0-B: apply post-flip window

**File:** `crates/gripsack-exec/src/apply.rs`

Move `render_env_file(&ctx.home, &generation.modules)?` to BEFORE
`store::flip(&ctx.home, next)?`. Final order:
`write_manifest → render_env_file → flip → adapters (warn-only)`.
Rationale: profile.sh references store paths (already published), not
the `current` symlink; rendering pre-flip makes a failure leave
NOTHING activated instead of an active-but-reported-failed
generation. Existing e2e cover env-file content; changelog note only.

## P0-C: enable GitHub issues

```
gh api repos/gripsack-dev/gripsack --jq '{has_issues}'
gh api -X PATCH repos/gripsack-dev/gripsack -f has_issues=true   # if false
```
If the restriction is org-level (interaction limits / member-only
issues), it needs the web UI — flag to Tarek. Verify anonymously:
`curl -s https://github.com/gripsack-dev/gripsack/issues | grep -c "restricted"`.

## P1-A: website copy (Tarek approves final)

- `website/index.html`: rephrase "no custom language" claim.
- Compare table: split chezmoi/Stow columns; drop "config linters with
  spans" row; shrink to killer rows (gradual adoption, mutable config
  ownership, rollback, packages+dotfiles, no system takeover).
- Build + push website repo.

## P1-B: linter long-tail — decision memo for Tarek (no unilateral change)

Options: (a) status quo, (b) showcase/contributed split + stop
auto-filing upstream-lag issues for contributed packs,
(c) archive non-showcase packs. Recommend (b). Prune the "29 planned
packs" line from the roadmap accordingly.

## P2: crash-recovery journal — write `plan/0017-deploy-journal.md`

Design sketch to elaborate: durable journal under
`$GRIPSACK_HOME/journal/<run-id>/` — per-destination intent records
(backup path + hash) fsync'd BEFORE mutation, commit marker after;
startup reconciliation completes or rolls back incomplete runs;
fault-injection e2e SIGKILLs grip at every boundary (pre-backup,
post-backup/pre-write, post-write/pre-commit, post-commit/pre-flip).
Evening scope: the plan doc + test-harness sketch only.

## Ship

0.17.14: P0-A + P0-B + changelog; PR → CI (audit/test/typescript-env)
→ admin merge → tag `core-v0.17.14` → release workflow → crates.io →
website auto-sync. P0-C is repo-settings, P1-A website repo push,
P1-B/P2 are docs/decisions.
