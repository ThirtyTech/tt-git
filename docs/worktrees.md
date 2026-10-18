# Working in a worktree

How to develop tt-git in a `git worktree` alongside the primary checkout:
where it goes, what it shares, what it must keep to itself, and how to run
and test it without touching the primary checkout.

## Where worktrees go

```
mygit/                      primary checkout, stays on main
├── .worktrees/
│   ├── graph-colors/       one folder per worktree, named after its branch
│   └── review-pr-12/
├── crates/
└── target/                 primary's build output (not shared)
```

Use `.worktrees/<name>` in the primary checkout. tt-git itself puts worktrees
there (`WORKTREE_DIR` in `crates/tt-git-core/src/repo.rs`), so worktrees made
by hand, by Claude, or from inside tt-git all end up in one place, and the
dev build exercises the same layout users get.

Nesting is safe for Cargo: the primary workspace's `members = ["crates/*"]`
doesn't reach `.worktrees/*/crates`, and each worktree has its own root
`Cargo.toml` with a `[workspace]`, so Cargo stops there. Creating a worktree
from inside another one still lands in the primary's `.worktrees/`, because
`Repo::main_root()` resolves through the shared git dir.

`/.worktrees/` must be ignored. tt-git adds it to `.git/info/exclude` when
*it* creates a worktree, but a plain `git worktree add` doesn't, so it also
goes in `.gitignore` (see [Changes to make](#changes-to-make)).

Editors that don't read `.gitignore` for indexing or file watching (VS Code
search and watchers, Windows Defender scans) should exclude `.worktrees/` too.

### Branches

A worktree needs either a branch or a detached HEAD. Use the branch name the
user asked for; don't invent one. With no branch requested, use `--detach`
(fine for reviews, bisecting and experiments). A branch can be checked out in
only one worktree at a time, so `main` stays in the primary checkout.

## What is shared and what is separate

| Thing | Shared? | Notes |
|---|---|---|
| Objects, branches, tags, remotes | Shared | One `.git` for all worktrees. A commit in one is visible to all immediately. |
| `.git/config` | Shared | Includes `tt-git.baseBranch`: picking a base with `b` in one worktree changes it for all. |
| Stash (`refs/stash`) | Shared | A stash made in a worktree shows up in the primary. |
| Hooks (`.git/hooks`) | Shared | |
| HEAD, index, merge/rebase/bisect state | Separate | Lives in `.git/worktrees/<name>/`. |
| Working tree files, `.snap.new` pending snapshots | Separate | |
| `~/.cargo` registry and git caches, rustup toolchain, `cargo-insta`, `cargo-nextest` | Shared | Nothing re-downloads. |
| `target/` | **Separate** | See below. |
| tt-git config and state (`config.toml`, `plugins.json`, logs, plugin state) | **Separate** | See below. Shared by default, which is wrong for a worktree. |
| Plugin API socket | Separate already | Named `tt-git-<pid>-<nonce>`, unique per process. Never export `TTGIT_SOCKET_PATH` in a dev shell. |

### `target/`: one per worktree

Keep Cargo's default, `<worktree>/target`. Don't point several checkouts at
one `CARGO_TARGET_DIR`:

- Cargo locks the build directory, so builds in two checkouts would wait on
  each other.
- On Windows a running `tt-git.exe` can't be overwritten, so a dev build left
  open in one checkout blocks linking in the other.
- Integration tests run `env!("CARGO_BIN_EXE_tt-git")`. With a shared target,
  a test in one worktree can run a binary the other one just linked.

The price is disk and a cold first build: the primary's `target/debug` is
about 16 GB. Delete a worktree's `target/` (or the whole worktree) when it's
done. If cold builds hurt, `sccache` (`RUSTC_WRAPPER=sccache`) shares compiled
dependencies across checkouts without sharing a target dir.

### tt-git config and state: one per worktree

By default every build reads `%APPDATA%\tt-git` (Windows) or
`~/.config/tt-git` and the matching state dir, the same files your installed
tt-git uses. From a worktree that means:

- `plugin link` writes the worktree's absolute path into the shared
  `plugins.json`; the primary and your real tt-git then run plugins from a
  folder that disappears when the worktree is removed.
- Both builds append to the same `<state>/logs/tt-git.log`.
- Config experiments leak into your daily tt-git.

So each worktree points `TTGIT_CONFIG_DIR` and `TTGIT_STATE_DIR` at its own
`target/dev/`, which is ignored and gets deleted with the worktree. Today
that's set by hand (see below); [Changes to make](#changes-to-make) proposes
having Cargo set it automatically.

## Lifecycle

PowerShell first, since that's the main dev shell; sh equivalents follow.

### Create

From the primary checkout:

```powershell
git worktree add .worktrees/graph-colors -b graph-colors main   # new branch, when asked for one
git worktree add --detach .worktrees/review main                # no branch
cd .worktrees/graph-colors
$env:TTGIT_CONFIG_DIR = "$PWD\target\dev\config"
$env:TTGIT_STATE_DIR  = "$PWD\target\dev\state"
cargo build
```

```sh
git worktree add .worktrees/graph-colors -b graph-colors main
cd .worktrees/graph-colors
export TTGIT_CONFIG_DIR=$PWD/target/dev/config
export TTGIT_STATE_DIR=$PWD/target/dev/state
cargo build
```

### Run

```powershell
cargo run -p tt-git                          # opens the worktree itself
cargo run -p tt-git -- C:\path\to\scratch     # opens a throwaway repo
cargo run -p tt-git -- --no-plugins
Get-Content target\dev\state\logs\tt-git.log -Wait   # logs, from a second terminal
```

Running against the worktree itself is useful: it's a linked worktree, so it
exercises the `git_dir` ≠ `common_dir` code paths. But the repo behind it is
the real one. Checkout, branch delete, rebase, stash, reset and picking a base
branch all act on the shared `.git` and are visible from the primary. For
anything destructive, open a scratch repo instead.

### Test

The test suites are already hermetic: `tt-git-testkit`'s `Sandbox` gives every
test a temp repo, a fake `HOME`, its own global gitconfig, its own
`TTGIT_CONFIG_DIR`/`TTGIT_STATE_DIR`, and scrubs `GIT_DIR` and friends (which
matters when tests run from a git hook inside a worktree). Run the same
command CI runs, from the worktree root:

```powershell
cargo insta test --workspace --test-runner nextest --check --unreferenced reject
cargo test --workspace --doc
```

This runs fine at the same time as the primary's tests; they only compete for
CPU. Snapshot review (`cargo insta review`) only touches the worktree's own
`.snap` files.

One exception: the ignored perf test (`crates/tt-git/tests/perf.rs`) uses a
fixed `%TEMP%\tt-git-perf-config`, so two perf runs at once collide (see
[Changes to make](#changes-to-make)).

### Merge back

Commit in the worktree when asked, then merge from the primary checkout:

```powershell
cd ../..                 # back to the primary checkout
git merge graph-colors
```

### Remove

Close anything holding files in the worktree first (a running dev build,
rust-analyzer, a terminal whose current directory is inside it); on Windows an
open handle makes removal fail halfway.

```powershell
git worktree remove .worktrees/graph-colors   # deletes ignored files like target/ too
git branch -d graph-colors                    # once merged
git worktree prune                            # clears records of folders deleted by hand
```

`git worktree remove` refuses if there are uncommitted or untracked files.
Check what they are before reaching for `--force`.

## Changes to make

What it takes to make the above the default rather than a checklist:

1. **Ignore `.worktrees/`.** Add `/.worktrees/` to `.gitignore`, so hand-made
   worktrees don't show up as untracked in the primary.
2. **Isolate dev config and state automatically.** Add `.cargo/config.toml`:

   ```toml
   [env]
   TTGIT_CONFIG_DIR = { value = "target/dev/config", relative = true }
   TTGIT_STATE_DIR = { value = "target/dev/state", relative = true }
   ```

   `relative = true` resolves against the checkout that contains `.cargo/`,
   so every worktree (and the primary) gets its own dirs under `target/` with
   no shell setup. A variable already set in the shell still wins. Tests are
   unaffected, since `Sandbox` sets both explicitly. This changes the
   documented default ("a dev build reads your real config"): `cargo run`
   would start with an empty config and no plugins until you copy or link
   them, so it needs a decision before it lands. The README's Development
   section would change to match.
3. **Give the perf test a unique config dir.** Replace the fixed
   `temp_dir().join("tt-git-perf-config")` with a `tempfile::tempdir()`.
4. **Point to this page.** Link it from the README's Development section.
   (`CLAUDE.md` already links it, and covers how agents build into
   `target/agent` so they never disturb a running dev copy.)
5. **Optional: `sccache`.** Only if cold builds in new worktrees turn out to
   be a real cost.
