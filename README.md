# tt-git

A terminal git client in Rust ([ratatui](https://ratatui.rs)), inspired by
[lazygit](https://github.com/jesseduffield/lazygit) and
[gitui](https://github.com/gitui-org/gitui), with:

- **A GitLens-style commit graph as the main view.** It shows `git log --all`
  with stable, colored lanes and badges for branches, remotes and tags.
  Press `a` to switch between all branches and just the current branch (plus
  its upstream).
- **A status pane** like lazygit's files panel: stage, unstage, discard,
  commit and amend, with the diff in the details pane. It also handles
  merge/rebase/cherry-pick conflicts.
- **One branch list for local and remote branches**, sorted by recent
  activity (commits, fetches, checkouts). If you check out a branch that only
  exists on a remote, tt-git creates the local tracking branch for you
  without asking.
- **Plugins built the same way as [herdr](https://herdr.dev/docs/plugins/):**
  plain subprocesses with a TOML manifest. Plugins can add actions, event
  hooks, startup commands and terminal panes, and they talk back through the
  `tt-git` CLI or a JSON socket. See [docs/plugins.md](docs/plugins.md).

```
╭[1] Status: repo · main → origin/main ↑1╮╭[3] Graph [all] 1/5────────────────────────────────────╮
│ M src/app.rs                           ││●    [side] side experiment          Test User   2h b66 │
│?? notes.md                             ││◉─╮   main  <v1.0> Merge 'feature'   Test User   3h 59f │
╰────────────────────────────────────────╯││ ●  [feature] feature work           Test User   1d 788 │
╭[2] Branches 3──────────────────────────╮│● │  main work                       Test User   1d f75 │
│* main            ✓          2h         ││●─╯  base                            Test User   2d b29 │
│  feature         local      1d         │╰────────────────────────────────────────────────────────╯
│  their-feature   origin     3d         │╭[4] Commit 59f0f13──────────────────────────────────────╮
╰────────────────────────────────────────╯│commit 59f0f13…  Author: …  + diff                      │
 a all/current  n new branch  M merge into current  r rebase  C cherry-pick  tab next pane  ? help
```

## Install

You need git 2.31 or newer on `PATH`. To build from source:

```sh
cargo install --path crates/tt-git
tt-git            # in a repository, or: tt-git path/to/repo
```

## Keys

Press `?` for the keys of the focused pane, or `:` (or `ctrl+p`) for the
command palette, which lists built-in commands and plugin actions.

| Where | Key | Does |
|---|---|---|
| anywhere | `1` `2` `3` `4` / `tab` `shift+tab` | focus status / branches / graph / details |
| | `j` `k` `↑` `↓`, `ctrl+d` `ctrl+u`, `g` `G` | move, page, top/bottom |
| | `P` / `p` | push (sets upstream on first push) / pull |
| | `ctrl+r` | refresh |
| | `:` `ctrl+p` | command palette |
| | `q` | quit |
| graph | `a` | all branches ↔ current branch |
| | `n` | new branch at the selected commit |
| | `M` / `r` / `C` | merge into current / rebase current onto / cherry-pick |
| branches | `enter` `space` | check out (remote-only branches get a tracking branch) |
| | `n` `d` `R` | new / delete (asks before force-deleting unmerged) / rename |
| | `f` | `fetch --all --prune` |
| | `/` | filter as you type (`esc` clears) |
| | `M` / `r` | merge / rebase onto the selected branch |
| status | `space` `enter` | stage / unstage the file |
| | `a` | stage everything (or unstage, if all staged) |
| | `d` | discard changes to the file (asks first) |
| | `c` / `A` | commit / amend keeping the message |
| | `C` / `X` | continue / abort a merge, rebase or cherry-pick |
| details | `o` | open the link in view with a plugin link handler |

The mouse also works: click a pane or row to focus and select it, use the
wheel to scroll the pane under the pointer, and `ctrl`+click a link in the
details pane. Every binding can be changed; see
[docs/configuration.md](docs/configuration.md).

## Plugins

```sh
tt-git plugin install owner/repo          # from GitHub; shows what will run and asks first
tt-git plugin link ./my-plugin            # local development
tt-git plugin list
```

[docs/plugins.md](docs/plugins.md) covers the manifest format, environment
variables, events, panes, the socket API and the CLI.
[examples/plugins](examples/plugins) has working examples.

## Development

The workspace is split into crates:

| Crate | What |
|---|---|
| `tt-git-core` | git CLI runner, parsers, unified branch list, write operations |
| `tt-git-graph` | lane layout for the commit graph (no git or UI dependencies) |
| `tt-git-plugin` | manifest, registry, runtime, install/GC, NDJSON socket |
| `tt-git-tui` | the ratatui app: panes, keymap, modals, PTY plugin panes |
| `tt-git` | the binary and CLI |
| `tt-git-testkit` | sandbox repos, the in-process `TestDriver`, plugin fixtures |

```sh
cargo test --workspace            # unit, integration, in-process UI and PTY end-to-end tests
cargo insta review                # after intentional UI changes
cargo bench -p tt-git-graph       # graph layout on a synthetic 100k-commit history
```

Tests run against real git in throwaway repositories with a fake `$HOME`
and fixed dates (`Sandbox`). UI flows drive the real app on ratatui's
`TestBackend` and wait until the app is idle after each key (`TestDriver`),
so they never sleep. A few end-to-end tests run the real binary in a
pseudo-terminal, ConPTY on Windows, via [termlens](https://crates.io/crates/termlens).
