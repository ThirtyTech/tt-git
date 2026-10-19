# tt-git: keeping the docs in step

The user documentation lives in `docs-site/` (Starlight on Astro, published
to <https://thirtytech.github.io/tt-git/>). Its README covers building it and
the screenshot pipeline.

## The rule

A change to anything a user can see or rely on isn't finished until the docs
describe it. Update them in the same change, not as a follow-up. That covers:

- keys and their defaults, in any pane or dialog
- what a command, pane, dialog or prompt does, including the text it shows
  and when it asks before acting
- config settings and the layout tree
- CLI commands, options and environment variables
- plugin events and their data, socket methods, manifest fields and the
  plugin environment
- which git commands tt-git runs for an operation

A pure refactor, a test-only change or a bug fix that makes the app do what
the docs already say needs no doc change. Say so when you report back.

## Where things are documented

Most features appear on more than one page. Check every place in the row.

| You changed | Update |
|---|---|
| A key or its default | `reference/keys.mdx` (by pane), the pane's own page (its key table and its `## Keys` section), the Commands table in `configuration/keys.mdx` for a new command, the README key table, the table in `docs/configuration.md` |
| What a pane does | `panes/<pane>.mdx`; `getting-started/tour.mdx` if the first screen changes |
| A git operation (checkout, stash, merge, push…) | the page for it under `git/` or `panes/`, plus `reference/how-it-works.mdx` for the git commands it runs |
| A config setting | `configuration/index.mdx`, `docs/configuration.md` |
| Layout | `configuration/layout.mdx` |
| The CLI | `reference/cli.mdx` |
| Plugin events | `plugins/events.mdx`, the events table in `docs/plugins.md` |
| Socket API, manifest, plugin environment | `plugins/api.mdx`, `plugins/manifest.mdx`, `plugins/environment.mdx`, `docs/plugins.md` |
| The help popup, palette or help bar | `getting-started/navigation.mdx`, `git/palette.mdx`, `getting-started/tour.mdx` |

Paths above are under `docs-site/src/content/docs/`. A new page also needs a
sidebar entry in `docs-site/astro.config.mjs`.

To find every place a feature is mentioned, search for its key, its label and
the git command behind it:

```sh
grep -rn -e "cherry-pick" -e "<kbd>C</kbd>" README.md docs docs-site/src/content
```

## Writing the pages

- Match the page you're editing: short sentences, second person, a key table
  per pane, keys as `<kbd>x</kbd>`.
- Link between pages with absolute paths without the `/tt-git` base, like
  `[Stash](/panes/stash/)`. `astro.config.mjs` adds the base.
- Describe what happens, including the failure paths a user will hit (asks
  first, refuses, restores something).

## Screenshots

The images in `docs-site/src/assets/screens/` are captured from the real app.
If your change alters what one of them shows (a new help bar entry, a new
dialog, different pane contents), regenerate it as `docs-site/README.md`
describes. A new dialog worth showing gets a capture in
`crates/tt-git-testkit/examples/docshots.rs` and an entry in
`docs-site/screens/shots.mjs`.

The capture step runs cargo, so the [CLAUDE.md](CLAUDE.md) build rules apply.
Run it from the checkout root with the agent target dir instead of
`pnpm screens:capture`:

```sh
CARGO_TARGET_DIR=target/agent cargo run -p tt-git-testkit --example docshots -- docs-site/screens/captures
cd docs-site && pnpm screens
```

Look over the new images before committing them. If you can't regenerate
them (no pnpm or Chrome), say which ones are now out of date.

## Checking

From `docs-site/`, `pnpm build` must pass; CI builds the site on every pull
request that touches it. When you report back, list the doc pages you
changed, or say why none needed changing.
