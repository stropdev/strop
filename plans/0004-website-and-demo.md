# 0004 — Website & Demo Machinery

> The strop.dev site and the demo-GIF pipeline, borrowed from rootle.dev/gripsack.dev
> and simplified where strop's nature allows. The demos are the product's first
> impression — they must show the operator-pending preview, the thing nobody else has.

Status: accepted-in-principle. Site scaffolded at placeholder stage; demo pipeline lands
with M0 (there is nothing to tape until the prototype exists).

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

## 2. Site repo (simplified vs. rootle/gripsack)

Both predecessors carry a `build.py` that converts `doc/*.md` into doc pages with a
shared rail. Strop has no docs to mirror yet — **the site starts as pure static files**
(`index.html`, `assets/site.css`, `img/`), deployed as-is. No build step, no Python, no
uv. When real docs exist (M1-era), adopt the rootle build.py pattern verbatim — it is
proven; do not reinvent it then either.

Placeholder-stage content: brand, tagline (*see the cut before you make it*), the
one-line lineage (Neovim's hands, Helix's spine, rootle's eyes, GitLens' memory), an
"in the forge" status chip, GitHub link. Demo slot and install instructions exist in
markup but stay hidden until the first release/GIF lands. The version chip is stamped
client-side from the releases API until build.py exists (then build-time, rootle
pattern).

House style applies to the site too (plan 0003 §5): one accent color, desaturated dark
base, clean Unicode, zero animation theater (a blinking caret is allowed; it is the
editor's heartbeat, not decoration).

DNS (manual, one-time, Porkbun): apex `strop.dev` → GitHub Pages A records
(185.199.108-111.153), `www` → CNAME `stropdev.github.io`; repo carries the `CNAME`
file. HTTPS enforced once the cert provisions.

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
editor; a demo that lies is a bug (same doctrine as the preview resolver).

## 5. Deferred

- build.py + docs mirroring (when docs exist, §2).
- Palette-variant renders + site palette picker (theme engine prerequisite, §3).
- install.sh served from the site (lands with the first real release; rootle/gripsack
  pattern: curl | sh resolving the right tarball per platform).
- Website repo for strop.dev *marketing* pages beyond the landing (post-M5).
