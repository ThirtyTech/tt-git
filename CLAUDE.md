# tt-git: notes for agents

A Rust/ratatui git TUI. Cargo workspace under `crates/`. Developer docs:
README "Development", [docs/configuration.md](docs/configuration.md),
[docs/plugins.md](docs/plugins.md), [docs/worktrees.md](docs/worktrees.md).

Every user-visible change also updates the doc site in `docs-site/`; see
[AGENTS.md](AGENTS.md) for where each kind of change is documented.

@AGENTS.md

## Build and test without touching the user's running tt-git

The user often has `cargo run -p tt-git` open in this checkout for their own
testing. That process runs `target/debug/tt-git.exe` and uses their real
tt-git config, plugins and logs. Nothing you do may disturb it.

### 1. Build into `target/agent`, always

Every cargo command you run (build, run, test, insta, clippy, doc) uses
`CARGO_TARGET_DIR=target/agent`. Never build into `target/debug`.

- On Windows the running `target/debug/tt-git.exe` is locked. Building there
  fails with "Access is denied" / "failed to remove file", or worse,
  succeeds after the user quits and silently swaps the binary they're testing.
- Builds in the same target dir wait on each other's Cargo lock.
- Integration tests run `env!("CARGO_BIN_EXE_tt-git")`, which is
  `target/agent/debug/tt-git.exe` under this rule, so tests always run your
  build.

Shell state doesn't carry between tool calls, so set it on every command. A
relative `CARGO_TARGET_DIR` resolves against the current directory, so run
cargo from the checkout root.

```sh
CARGO_TARGET_DIR=target/agent cargo test -p tt-git-core
```

```powershell
$env:CARGO_TARGET_DIR = 'target/agent'; cargo test -p tt-git-core
```

The first build of `target/agent` is cold (about 20 s for the binary, longer
once tests build). Don't seed it by copying `target/debug`.

If anything mentions a lock or an access error on `target/debug`, you forgot
`CARGO_TARGET_DIR`. Fix the command; don't work around the lock.

### 2. Give your binary its own config and state

When you run tt-git yourself (not through tests), also set:

```
TTGIT_CONFIG_DIR=target/agent/dev/config
TTGIT_STATE_DIR=target/agent/dev/state
```

Otherwise your copy reads and writes the user's `%APPDATA%\tt-git`:
`plugins.json`, plugin state, and the same `logs/tt-git.log` their running
instance is writing. Tests don't need this; `tt-git-testkit`'s `Sandbox`
already isolates them.

### 3. Clear `TTGIT_SOCKET_PATH` and `TTGIT_REPO_PATH`

If these are set (for example because you were started from a tt-git plugin
pane), your binary's CLI commands go to the user's running instance through
its API socket instead of doing their own thing. Unset them:
`env -u TTGIT_SOCKET_PATH -u TTGIT_REPO_PATH ...` in Bash,
`Remove-Item Env:TTGIT_SOCKET_PATH, Env:TTGIT_REPO_PATH -ErrorAction Ignore`
in PowerShell.

### 4. Don't point your binary at this repo for anything that writes

The user's instance watches this repo and refreshes when it changes. Checkout,
commit, stash, rebase, branch delete, reset and picking a base branch from
your copy would all show up in theirs (and `tt-git.baseBranch` is shared git
config). Use a scratch repo in your scratchpad, or let tests make sandboxes.
Read-only use against this repo is fine.

### 5. Never stop or clean up after the user's process

Don't kill `tt-git.exe` or `cargo` processes you didn't start. Don't delete or
`cargo clean` `target/debug`. `cargo clean` is fine only with
`CARGO_TARGET_DIR=target/agent` set.

## Commands

Run from the checkout root. Bash form; in PowerShell set
`$env:CARGO_TARGET_DIR` first as shown above.

```sh
# Everything CI checks (CI also passes --test-runner nextest; nextest isn't installed here)
CARGO_TARGET_DIR=target/agent cargo insta test --workspace --check --unreferenced reject
CARGO_TARGET_DIR=target/agent cargo test --workspace --doc

# One crate, or one test by name
CARGO_TARGET_DIR=target/agent cargo test -p tt-git-tui
CARGO_TARGET_DIR=target/agent cargo test -p tt-git --test e2e help_popup

# Lint and format
CARGO_TARGET_DIR=target/agent cargo clippy --workspace --all-targets
cargo fmt --all

# Snapshot changes: review the .snap.new files, accept only what you meant to change
CARGO_TARGET_DIR=target/agent cargo insta test -p tt-git-tui
CARGO_TARGET_DIR=target/agent cargo insta accept
```

### Running your build

CLI subcommands, with the isolation from rules 2 and 3:

```sh
env -u TTGIT_SOCKET_PATH -u TTGIT_REPO_PATH \
  TTGIT_CONFIG_DIR=target/agent/dev/config TTGIT_STATE_DIR=target/agent/dev/state \
  CARGO_TARGET_DIR=target/agent cargo run -p tt-git -- plugin list
```

The TUI needs a real terminal, so you can't drive it from a plain shell
command. To check UI behavior, use the existing harnesses:

- `crates/tt-git/tests/e2e.rs` runs the real binary in a ConPTY via
  `termlens` against a `Sandbox` repo and asserts on the screen. Add a test
  there for end-to-end checks.
- `crates/tt-git-tui/tests/` render the app into insta snapshots. Faster, and
  enough for layout and key-handling changes.

## Worktrees

[docs/worktrees.md](docs/worktrees.md) covers worktrees in full. In short:
they go in `.worktrees/<name>`. Use a branch only when the user names one,
otherwise `--detach`. The rules above still apply inside a worktree, since the
user may be running `cargo run` there too.
