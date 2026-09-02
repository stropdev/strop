# 0004 — Website & Demo Machinery

> The strop.dev site and the demo-GIF pipeline, borrowed from rootle.dev/gripsack.dev
> and simplified where strop's nature allows. The demos are the product's first
> impression — they must show the operator-pending preview, the thing nobody else has.

Status: live foundation. Placeholder site deployed (logo, palettes, demo GIF, HTTPS
enforced); demo pipeline green end-to-end (push → musl build → VHS render → artifacts
PR → site publish → redeploy dispatch). What follows is the road from placeholder to
the real thing.

---

## 1. Repos and flow

```
stropdev/strop                 demos/*.tape, .github/workflows/demo.yml
        │  demo.yml renders GIFs (VHS), publishes to site repo
        ▼
stropdev/stropdev.github.io    index.html + assets, img/*.gif, pages.yml → GitHub Pages
        │
        ▼
strop.dev (Porkbun DNS → GitHub Pages, CNAME file in repo)
```

Same shape as rootle/gripsack: app repo owns tapes and rendering; site repo owns
presentation; `repository_dispatch: rebuild` + `SITE_REPO_TOKEN` is the coupling;
release.yml pings the site so the version chip stays current.

## 2. Site repo — the stages

**Stage 1 (done): pure static.** `index.html`, `assets/site.css`, `img/` — no build
step. Palette picker is client-side CSS vars; the inline logo SVG is painted from the
same vars, so it re-themes live with zero rebuilds (simpler than rootle's build-time
swap — keep it).

**Stage 2 (lands with real docs): adopt gripsack's `build.py`.** The moment we have
user docs worth reading — keymap reference, motions guide, config reference,
regex-divergences table (0001 §2.5) — the site grows a docs section, and we adopt the
proven machinery rather than inventing:

- `website/build.py` (`uv run --with markdown python website/build.py` → `public/`):
  `doc/*.md` → `docs/<slug>.html` with the shared chrome — left rail (brand, nav,
  on-this-page TOC), content column.
- **Changelog page fetched from the app repo at build time** (with committed fallback) —
  the changelog lives with the code, the site never drifts.
- **Doc-local SVG diagrams inlined and re-themed** with the palette picker (an `<img>`
  can't inherit page CSS; inline and map palette hexes to CSS vars).
- The version chip flips from client-side fetch to build-time stamp (deterministic).

**Stage 3 (post-M5): marketing pages beyond the landing**, as needed.

### The polish bar (gripsack.dev, landed 2026-09-02 — steal all of it)

- **Logo dissolves into the hero glow**: feathered tile edge (blurred-rect mask applied
  in `build.py`; README keeps the static bake). No hard-edged backdrop tiles.
- **Demo slider, not a wall of GIFs**: the demo set (§4) becomes one window with tabs —
  `the cut` / `picker` / `git` / `jumplist` — each tab a tape. First tab eager, the rest
  lazy with true dimensions (zero CLS).
- **De-cram rules**: spacing rhythm `clamp(56px, 8vw, 96px)` between sections; hairline
  section fades; quiet metadata chips; window shadows + hover states.
- **Motion discipline**: one-shot scroll-in reveal, fully disabled under
  `prefers-reduced-motion` and no-JS. The site's one allowed loop is the caret blink —
  the editor's heartbeat.
- **A11y is part of pretty**: global `:focus-visible` ring, real alt text, keyboard-
  operable tabs.
- **Subtle scrollbars everywhere** (thin, transparent-track) — the chunky default
  scrollbar next to a nav rail is the first thing the eye lands on.

House style (0003 §5) governs as before: one accent, desaturated dark base, clean
Unicode, no latency theater.

DNS (done 2026-09-02): Porkbun API records landed, apex + www → GitHub Pages, cert
issued, HTTPS enforced. Cert-issuance lesson: if GitHub stalls on a correct domain,
remove and re-add the custom domain (PUT pages cname) — that kicks provisioning.

## 3. Demo pipeline (strop repo)

Borrowed from gripsack's demo.yml, simplified:

- **Trigger**: push to main touching `crates/**`, `demos/**`, `Cargo.toml/lock`, plus
  `workflow_dispatch`.
- **Build**: `docker compose run --build --rm -e VERSION=0.0.0-demo release` — the exact
  release artifact, so the demo can never drift from what ships (plan 0002).
- **Render**: `ghcr.io/charmbracelet/vhs` container, tape types real keystrokes into a
  real strop. Simplification vs. gripsack: no deno install, no trust-gate env, no
  fixture repo beyond a small demo file tree baked into `demos/`.
- **Publish**: idempotent artifacts PR on a force-pushed `demo/artifacts` branch (bot
  PRs don't trigger CI; admin-merge), then copy to `site/img/` + rebuild dispatch —
  the rootle/gripsack pattern, unchanged.
- **Tape conventions**: `FontSize 28`, `Width 1800`, `Height 1120` (gripsack numbers —
  they read well at README scale), `Type@40ms` with `Sleep` beats long enough to read
  each screen.

### Palette variants: show strop's own themes, not VHS's

**The contract for when themes land (recorded 2026-09-02):** the demo re-renders per
theme (`STROP_THEME=<name>` in the tape — the real theme engine, never VHS terminal
colors), one GIF per palette the site picker offers, and the site's palette picker
swaps the demo image to match (the rootle/gripsack pattern). A demo that shows a theme
the editor can't produce is a lie; a palette picker that doesn't re-theme the demo is
half-done. Both sides ship together.

Gripsack renders one GIF per VHS terminal theme (`Set Theme "Catppuccin Mocha"` etc.)
so the site's palette picker can swap the demo. That colors the *terminal*, and for a
CLI whose theme is the terminal's theme, it is honest. Strop is an editor with its own
theme engine — palette variants must be rendered by **setting strop's theme in the
tape** (`STROP_THEME=nord strop …` or config), with VHS's theme set to a neutral dark so
chrome doesn't fight content. Deferred until the theme engine and the site palette
picker exist; v1 renders the canonical default palette only. When it lands, the variant
loop is gripsack's `palettes.txt` shape with `STROP_THEME` substituted for `Set Theme`.

## 4. The demo set (what the tapes must show)

Ordered by persuasion; the first tape is the whole pitch:

1. **demo.tape — the soul.** Open a file, `ci[` with the live operator-pending preview,
   a `d/pat⏎` search motion with incsearch, a surround (`cs"'`), dot-repeat flash.
   Thirty seconds of "wait, it shows you the cut *before*?"
2. **demo-picker.tape** — `Space f`, `Space /` live grep, preview pane + dimmed backdrop,
   `enter` into the match.
3. **demo-git.tape** — `Space g l` commit browser, `enter` changed-files delta view,
   `Space g b` blame popup, `Space g y` permalink copied.
4. **demo-jump.tape** — `Space j` jumplist picker with the trail markers (`▲3`/`▼2`),
   `Tab`-cycling with the backdrop preview.

Each tape is a real session against fixture files in `demos/fixtures/` — no mocking the
editor; a demo that lies is a bug (same doctrine as the preview resolver). On the site,
the set ships as the tabbed slider (§2 polish bar): `the cut` / `picker` / `git` /
`jumplist`, first tab eager, rest lazy.

## 5. Deferred

- Stage 2 machinery: build.py, docs section, changelog fetch, tabbed demo slider (§2).
- Palette-variant renders + site palette picker (theme engine prerequisite, §3).
- install.sh served from the site (lands with the first real release; rootle/gripsack
  pattern: curl | sh resolving the right tarball per platform).
- Website repo for strop.dev *marketing* pages beyond the landing (post-M5).
