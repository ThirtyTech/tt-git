# tt-git docs site

User documentation for tt-git, built with
[Starlight](https://starlight.astro.build/) on [Astro](https://astro.build/).
It builds to static HTML in `dist/`.

## Local development

From this directory:

```sh
pnpm install
pnpm dev        # http://localhost:4321, reloads as you edit
pnpm build      # static site in dist/
pnpm preview    # serve dist/ with search (Pagefind) working
```

## Layout

```
src/
├── assets/           logo and screenshots (screenshots under assets/screens/)
├── content/docs/     one .md/.mdx file per page; the path is the URL
└── styles/theme.css  colors taken from the logo
astro.config.mjs      site title, logo, sidebar
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
