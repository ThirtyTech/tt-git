# tt-git docs site

User documentation for tt-git, built with
[Starlight](https://starlight.astro.build/) on [Astro](https://astro.build/).
It builds to static HTML in `dist/`.

## Local development

From this directory:

```sh
pnpm install
pnpm dev        # http://localhost:4321/tt-git/, reloads as you edit
pnpm build      # static site in dist/
pnpm preview    # serve dist/ with search (Pagefind) working
```

The site lives under `/tt-git/` (see Publishing), locally too.

## Publishing

`.github/workflows/docs.yml` builds the site on every push to `main` that
touches `docs-site/` and publishes it with GitHub Pages at
<https://thirtytech.github.io/tt-git/>. Pull requests only build it. The
repository's Settings → Pages → Source must be set to **GitHub Actions**.

Because it's a project site, everything is served under `/tt-git/`
(`site` and `base` in `astro.config.mjs`). Write links between pages as
absolute paths without the base, like `[Graph](/panes/graph/)` or
`<LinkCard href="/panes/graph/" />`; a small rehype plugin in
`astro.config.mjs` adds `/tt-git` to them. Links in frontmatter (the landing
page's hero buttons) don't go through it, so they're relative.

## Layout

```
src/
├── assets/           logo and screenshots (screenshots under assets/screens/)
├── content/docs/     one .md/.mdx file per page; the path is the URL
└── styles/theme.css  colors taken from the logo
public/               favicon, PNG icons and the link-preview card (og.png)
astro.config.mjs      site title, logo, sidebar, meta tags
```

The sidebar is set in `astro.config.mjs`.

## Theme

The colors come from the logo (`src/assets/logo.svg`, a copy of
`docs/assets/logo.svg`): ThirtyTech cyan `#4bc0eb` → `#006698` as the accent,
silver-grey `#929291`, the near-black tile `#11151c` as the dark background,
and the merge node's green `#7bd938`. Dark mode is the default; light mode
uses the deep cyan for contrast.

## Screenshots

Every image in `src/assets/screens/` is the real app drawing a mock
repository, with highlights and numbered callouts added on top. Pages show
them with `<Screenshot name="…" alt="…" />` (`src/components/Screenshot.astro`).

1. **Capture.** `pnpm screens:capture` runs the `docshots` example in
   `crates/tt-git-testkit/examples/`. It builds a mock project (`lumen-web`:
   five authors, a few months of history, merged and unmerged branches,
   tags, stashes, uncommitted changes, a merge conflict), drives the app with
   the same `TestDriver` the UI tests use, and writes every screen as JSON to
   `screens/captures/` (ignored by git).
2. **Render.** `pnpm screens` (or `pnpm screens graph` for the shots whose
   name contains `graph`) draws each capture at 1920×1080 in Chrome via
   playwright-core, adds the annotations from `screens/shots.mjs`, and saves
   `src/assets/screens/<name>.webp`. It uses your installed Chrome; set
   `CHROME_PATH` to use another Chromium.

`screens/shots.mjs` describes each image: which capture, the whole screen or
a zoomed region, and callouts that point at text found on screen (so they
survive small layout changes). After a UI change, run both steps and look
over the images before committing them.

### The desktop app

The `gui-*` images are the desktop app's real windows, not renders of a
capture. From the checkout root (with `CARGO_TARGET_DIR=target/agent` and
the other build rules in `CLAUDE.md` when you're an agent):

1. **Capture.** `pnpm screens:gui:capture` runs the `guishots` example in
   `crates/tt-git-gui/examples/`. It opens the app's windows on the same
   `lumen-web` mock (`tt_git_testkit::mock`), drives them, and saves each
   window as a PNG in `screens/gui-captures/` (ignored by git), named after
   the system it ran on: `overview-windows.png`.
2. **Convert.** `pnpm screens:gui` (or `pnpm screens:gui menu`) saves them
   as `src/assets/screens/gui-<name>-<os>.webp`, losslessly. No callouts.

Each system takes its own: GPUI draws a window to an image on Windows and
macOS, so run the capture on each of them. On Linux, take the window with
the desktop's screenshot tool and save it under the same name. Pages show
the shots for a system under its tab in [the desktop app's
overview](src/content/docs/desktop/index.mdx).

## Link previews

When someone pastes a link to the site in Discord, Slack or X, the preview
shows the page's title and description, the card in `public/og.png` and the
`theme-color` (cyan) down the side. The tags are the `head` list in
`astro.config.mjs`; Starlight adds the title, description and URL per page
from each page's frontmatter, so give every page a `description`.

`pnpm screens:social` (`screens/social.mjs`) draws `og.png` from the logo and
`src/assets/screens/hero.webp`, and the PNG icons (`apple-touch-icon.png` and
`icon-192.png`) from `public/favicon.svg`. Run it after the
hero shot or the logo changes.
