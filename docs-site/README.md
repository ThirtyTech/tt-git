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
