# tt-git

A terminal git client in Rust ([ratatui](https://ratatui.rs)), inspired by
[lazygit](https://github.com/jesseduffield/lazygit) and
[gitui](https://github.com/gitui-org/gitui), with:

- **A GitLens-style commit graph as the main view.** It shows `git log --all`
  with stable, colored lanes and badges for branches, remotes and tags.
  Press `a` to switch between all branches and just the current branch (plus
  its upstream).
- **A status pane** like lazygit's files panel: stage, unstage, discard,
  commit and amend. It also handles merge/rebase/cherry-pick conflicts.
- **One branch list for local and remote branches**, sorted by recent
  activity (commits, fetches, checkouts). If you check out a branch that only
  exists on a remote, tt-git creates the local tracking branch for you
  without asking.
  Two more tabs list the branches merged and not yet merged into the
  project's primary branch (or any base branch you pick). On those tabs the
  graph shows only the selected branch, from where it branched off to where
  it was merged back in.
- **Plugins built the same way as [herdr](https://herdr.dev/docs/plugins/):**
  plain subprocesses with a TOML manifest. Plugins can add actions, event
  hooks, startup commands and terminal panes, and they talk back through the
  `tt-git` CLI or a JSON socket. See [docs/plugins.md](docs/plugins.md).

```
╭[3] Graph [all] 1/5─────────────────────────────────────────────────────────────────────────────╮
│●    [side] side experiment                                             Test User      2h b66   │
│◉─╮   main  <v1.0> Merge 'feature'                                      Test User      3h 59f   │
││ ●  [feature] feature work                                             Test User      1d 788   │
│● │  main work                                                          Test User      1d f75   │
│●─╯  base                                                               Test User      2d b29   │
╰────────────────────────────────────────────────────────────────────────────────────────────────╯
╭[1] Status: repo · main → origin/main ↑1───────╮╭[2] Branches 3─────────────────────────────────╮
│ M src/app.rs                                  ││* main            ✓          2h                │
│?? notes.md                                    ││  feature         local      1d                │
╰───────────────────────────────────────────────╯╰───────────────────────────────────────────────╯
 enter files + diff  space show/hide details  a all/current  n new branch  tab next pane  ? help
```

Press `space` on a commit to open its details in place of the bottom row,
or `enter` to open it fullscreen as changed files beside a diff.
The layout is configurable; see [docs/configuration.md](docs/configuration.md#layout).

## Install

You need git 2.31 or newer on `PATH`. To build from source:

```sh
cargo install --path crates/tt-git
tt-git            # in a repository, or: tt-git path/to/repo
```

## Keys

Press `?` for the keys of the focused pane (type to search them, `esc` to
close), or `:` (or `ctrl+p`) for the
command palette, which lists built-in commands and plugin actions.

| Where | Key | Does |
|---|---|---|
| anywhere | `1` `2` `3` `4` / `tab` `shift+tab` | focus status / branches / graph / details |
| | `ctrl+h` `ctrl+j` `ctrl+k` `ctrl+l` | focus the pane to the left / below / above / right |
| | `+` | zoom: graph (+ details if open) full width → focused one fullscreen → back |
| | `j` `k` `↑` `↓`, `ctrl+d` `ctrl+u`, `g` `G` | move, page, top/bottom |
| | `P` / `p` | push (sets upstream on first push) / pull |
| | `ctrl+r` | refresh |
| | `:` `ctrl+p` | command palette |
| | `q` | quit |
| graph | `enter` | the commit fullscreen: changed files on the left, the selected file's diff on the right (`j`/`k` pick a file, `pgup`/`pgdn` scroll, `esc` back) |
| | `space` | show / hide the commit's details (they replace the bottom row) |
| | `esc` | hide details (with details hidden: clear the filter) |
| | `a` | all branches ↔ current branch |
| | `f` | filter: only one author's commits, or the commits that changed a file or folder (`tab` in the panel switches between the two; both can be set) |
| | `n` | new branch at the selected commit (`tab` in the dialog: create it in a worktree) |
| | `M` / `r` / `C` | merge into current / rebase current onto / cherry-pick |
| | `c` | copy the commit hash |
| branches | `enter` `space` | check out (remote-only branches get a tracking branch) |
| | `n` `d` `R` | new (`tab` in the dialog: create it in a worktree under `.worktrees/`) / delete (asks before force-deleting unmerged) / rename |
| | `f` | `fetch --all --prune` |
| | `/` | filter as you type (`esc` clears) |
| | `]` `[` | next / previous tab: all branches, unmerged into the base branch, merged into it |
| | `b` | pick the base branch (default: the project's primary branch) |
| | `M` / `r` | merge / rebase onto the selected branch |
| | `c` | copy the branch name |
| status | `space` `enter` | stage / unstage the file |
| | `a` | stage everything (or unstage, if all staged) |
| | `d` | discard changes to the file (asks first) |
| | `c` / `A` | commit / amend keeping the message |
| | `C` / `X` | continue / abort a merge, rebase or cherry-pick |
| details | `o` | open the link in view with a plugin link handler |
| | `esc` | close details |

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

You need Rust 1.89 or newer (the workspace uses edition 2024) and git 2.31
or newer on `PATH`.

### Running from source

```sh
cargo run -p tt-git                        # the repository in the current directory
cargo run -p tt-git -- path/to/repo        # another repository
cargo run -p tt-git -- --no-plugins        # without plugins or the API socket
cargo run -p tt-git -- --config dev.toml   # with a different config file
cargo run -p tt-git -- plugin list         # subcommands work the same way
```

Everything after `--` goes to tt-git rather than Cargo.

A dev build reads your real config and installed plugins. To keep it
separate, point the config and state directories somewhere under `target/`:

```sh
export TTGIT_CONFIG_DIR=$PWD/target/dev/config
export TTGIT_STATE_DIR=$PWD/target/dev/state
```

```powershell
$env:TTGIT_CONFIG_DIR = "$PWD\target\dev\config"
$env:TTGIT_STATE_DIR = "$PWD\target\dev\state"
```

### Logs

The UI owns the terminal, so tt-git logs to `<state>/logs/tt-git.log`
instead (see [docs/configuration.md](docs/configuration.md#logs) for where
`<state>` is). `TTGIT_LOG` sets the level as an
[`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
string, default `info`. Follow the log from a second terminal:

```sh
TTGIT_LOG=debug,tt_git_tui=trace cargo run -p tt-git
tail -f target/dev/state/logs/tt-git.log   # PowerShell: Get-Content target\dev\state\logs\tt-git.log -Wait
```

### Working on plugins

`plugin link` uses a plugin straight from its directory, so edits take
effect without reinstalling:

```sh
cargo run -p tt-git -- plugin link examples/plugins/open-on-github
```

[docs/plugins.md](docs/plugins.md#testing-a-plugin) covers testing plugins.

### Tests

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

### Before you push

CI checks formatting and lints, then runs the tests on Linux, macOS and
Windows with [cargo-nextest](https://nexte.st) and
[cargo-insta](https://insta.rs). To run the same checks locally:

```sh
cargo install cargo-nextest cargo-insta   # once
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo insta test --workspace --test-runner nextest --check --unreferenced reject
cargo test --workspace --doc
```

`--check` fails on any snapshot that doesn't match, and
`--unreferenced reject` fails on snapshot files no test uses, so delete
those when you remove a test.

### Crates

| Crate | What |
|---|---|
| `tt-git-core` | git CLI runner, parsers, unified branch list, write operations |
| `tt-git-graph` | lane layout for the commit graph (no git or UI dependencies) |
| `tt-git-plugin` | manifest, registry, runtime, install/GC, NDJSON socket |
| `tt-git-tui` | the ratatui app: panes, keymap, modals, PTY plugin panes |
| `tt-git` | the binary and CLI |
| `tt-git-testkit` | sandbox repos, the in-process `TestDriver`, plugin fixtures |
