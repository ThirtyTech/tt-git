# Plan: undo / redo through HEAD's reflog

Step back and forward through what you did to HEAD (checkouts, commits,
resets, rebases, merges, pulls, cherry-picks) with <kbd>z</kbd> / <kbd>Z</kbd>,
lazygit-style. tt-git asks first only when the step would lose work.

## Behaviour

- <kbd>z</kbd> (global) undoes the newest HEAD action not already undone.
  Pressing it again keeps stepping back.
- <kbd>Z</kbd> (global) redoes the newest undone action. Once you do something
  new after undoing, there's nothing left to redo (same as an editor).
- Both appear in the command palette ("Undo", "Redo") and the help popup.
- What an action becomes when undone / redone:

  | Reflog entry | Undo | Redo |
  |---|---|---|
  | `checkout: moving from A to B` | switch to `A` (detach at the entry's previous commit if `A` isn't a local branch any more / was a commit) | switch to `B` (same rule, using the entry's own commit) |
  | `commit…`, `reset: moving to…`, `merge …`, `pull…` (not `--rebase`), `cherry-pick…`, `revert…` | `reset --hard` to the commit before the entry | `reset --hard` to the entry's commit |
  | `rebase (start)` … `rebase (finish)` (also `rebase -i`, `pull --rebase`) | `reset --hard` to the commit before `(start)` | `reset --hard` to the `(finish)` commit |

  Entries whose commit is the same as the one before (e.g. `git stash`'s
  `reset: moving to HEAD`, an aborted rebase) are skipped. Anything else
  (`clone`, `rebase (pick)` outside a range, unknown verbs) is skipped.
- Refuses, with a message and no git write:
  - an operation is in progress (merge, rebase, cherry-pick, revert):
    "Finish or abort the rebase first" (point at <kbd>C</kbd> / <kbd>X</kbd>);
  - nothing to undo / redo: a notice, not an error popup;
  - the step would undo the first commit (`commit (initial)`, nothing older
    in the reflog) — "Can't undo the repository's first commit";
  - HEAD moved between planning and running (race with an outside git
    command): "HEAD moved; press z again".
- **Asks first only when it would lose work**, i.e. there are uncommitted
  changes (any status entry, untracked included, same test as checkout):
  - reset-type step: "N files have uncommitted changes. Stash them, undo
    <what>, and put them back?" → stash `--include-untracked`, reset, `stash
    pop`. If the pop conflicts, git keeps the stash; the notice says the
    changes are on the Stash tab.
  - checkout-type step: the existing checkout prompt wording ("Stash them
    and check out A?"); changes stay on the Stash tab, exactly like
    <kbd>o</kbd> / <kbd>enter</kbd> today.
  - If the reset / switch fails after stashing, pop the stash back (as
    `stash_and_checkout` does).
  - Clean tree: run straight away, no dialog. Undone commits aren't lost:
    <kbd>Z</kbd> brings them back and they stay in the reflog.
- Success notice: `✓ undo commit 1a2b3c4 "fix typo"` /
  `✓ redo checkout feature` etc. (from `GitOp::label`).

## How tt-git tells its own steps apart

Every undo/redo git write runs with `GIT_REFLOG_ACTION` set, so its reflog
entry starts with a marker the walker counts:

- reset: `GIT_REFLOG_ACTION="[tt-git undo]"` → entry
  `[tt-git undo]: updating HEAD` (`[tt-git redo]` for redo).
- switch: git uses `GIT_REFLOG_ACTION` verbatim as the whole message for
  `checkout`/`switch`, so set it to
  `[tt-git undo] checkout: moving from <cur> to <target>`. That keeps the
  from/to in the reflog, and `parse_checkouts` (branch recency) can strip the
  marker prefix so undone checkouts still count as checkouts.
- The stash push before a dirty undo is **not** marked (its
  `reset: moving to HEAD` is a no-op entry and gets skipped).

Verify both message shapes against real git in a core test (git 2.50 here;
the `switch` behaviour has been in `builtin/checkout.c` since `switch` exists).

## The walker (pure, in core)

Lazygit's algorithm, which handles undo-after-redo-after-undo correctly.
Walk HEAD's reflog newest first with `counter = 0`:

- entry starts with `[tt-git undo]` → `counter += 1`; `[tt-git redo]` →
  `counter -= 1`; continue.
- `rebase…/pull --rebase (finish|abort)` → remember its commit, skip forward to
  the matching `(start)`; the action is `Rebase { from: commit before start,
  to: finish commit }`.
- `(start)` with no finish → `RebaseInProgress` (refuse if the repo really is
  rebasing; otherwise ignore it).
- checkout / move entries → action with `from` = the next-older entry's
  commit (and branch names for checkouts), `to` = this entry's commit.
- An action with `from == to` is skipped without touching `counter`.
- **Undo**: the first action seen with `counter == 0` is the one to undo;
  otherwise `counter -= 1` and continue.
- **Redo**: an action with `counter == 0` → nothing to redo; `counter > 1` →
  `counter -= 1`, continue; `counter == 1` → redo it.
- Run out of entries (or reach the oldest one with no `from`) → nothing more
  to undo / first-commit refusal.

Read the reflog with
`git log -g --format=%H%x1f%gs --max-count=<N+1> HEAD --` (per-worktree HEAD,
so linked worktrees work). N = 500; one extra so the oldest entry has a `from`.
Only read on keypress, in a background job — not part of the refs snapshot.

## Code changes

### `tt-git-core`

- `parse.rs`: `UNDO_LOG_FORMAT` and `parse_reflog(&[u8]) -> Vec<ReflogEntry {
  oid, subject }>`; `parse_checkouts` strips a leading `[tt-git undo] ` /
  `[tt-git redo] ` before matching `checkout: moving from`.
- New `undo.rs` (exported from `lib.rs`):
  - `pub enum UndoDirection { Undo, Redo }`
  - `pub enum UndoTarget { Switch(String), Detach(String), Reset(String) }`
  - `pub struct UndoStep { direction, kind: &'static str /* "commit",
    "checkout", "rebase", "reset", "merge", "pull", "cherry-pick", "revert",
    "amend" */, summary: String /* label text */, target: UndoTarget,
    expect_head: String, reflog_from: String /* for the switch marker */ }`
  - `pub enum NoStep { Nothing(UndoDirection), InProgress(InProgress),
    FirstCommit }`
  - `pub fn plan(entries: &[ReflogEntry], dir, local_branch: impl Fn(&str)
    -> bool) -> Result<UndoStep, NoStep>` — the pure walker above. Branch
    existence comes in as a closure so the walker stays pure.
  - Unit tests on hand-written reflog subjects: commit/undo/undo/redo/redo
    sequences, undo then new commit (nothing to redo), checkout to a deleted
    branch (detaches), detached `from`, rebase and `pull --rebase` ranges,
    aborted rebase skipped, stash no-op entry skipped, `commit (initial)`,
    `commit (amend)`, marker counting with prefixed checkout markers.
- `repo.rs` / `ops.rs`:
  - `Repo::undo_step(dir) -> Result<Result<UndoStep, NoStep>>`: in-progress
    check first (`in_progress()`), read the reflog, `rev-parse HEAD` for
    `expect_head`, local-branch check via `show-ref --verify -q
    refs/heads/<name>` (or the refs the walker names, batched with
    `for-each-ref`), then `undo::plan`.
  - `Repo::apply_undo(&UndoStep, stash: bool) -> Result<String>`:
    `rev-parse HEAD` must equal `expect_head` (else `GitError::Parse("HEAD
    moved…")`); optional `stash_push(All)`; run `reset --hard <oid>` /
    `switch <branch>` / `switch --detach <oid>` through
    `self.git().clone().with_env([("GIT_REFLOG_ACTION", marker)])`; on failure
    pop the stash back (reuse the `stash_tip` before/after check); on success
    of a reset with a stash, `stash pop` — if that fails, return Ok with a
    note that the changes are in `stash@{0}`.
- `tests/ops.rs` (Sandbox): end-to-end undo/redo of commit, amend, checkout,
  hard reset, merge (ff and real), rebase; dirty-tree stash + pop; pop
  conflict leaves the stash; HEAD-moved refusal; marker message shapes
  (`git log -g --format=%gs -1`); branch recency still sees an undone
  checkout.

### `tt-git-tui`

- `keymap.rs`: `Command::Undo` ("undo"), `Command::Redo` ("redo");
  defaults `(Global, Undo, ["z"])`, `(Global, Redo, ["Z"])`. Neither key is
  used in any pane today; text prompts and the help search already swallow
  keys before the keymap.
- `app.rs`: add both to `PALETTE_BUILTINS`; `dispatch` handles them in every
  pane (like `Push`/`Pull`) → spawn `loaders::plan_undo(repo, dir)` →
  new `Action::UndoPlanned(Result<Result<UndoStep, NoStep>, String>)`.
  `apply` for it: `NoStep` → notice / error text above; dirty status →
  `Modal::confirm` with `GitOp::Undo { step, stash: true }`; clean →
  `run_op(GitOp::Undo { step, stash: false })`. Ignore a second press while a
  plan or undo job is in flight (`busy` already tracks the op label; add a
  `planning_undo` flag).
- `modal.rs`: `GitOp::Undo { step: UndoStep, stash: bool }`; `label()` →
  `"undo commit 1a2b3c4"`, `"redo checkout feature"`, `"stash and undo …"`.
- `loaders.rs`: `plan_undo` job, and `run_op` arm calling `apply_undo`.
- `op_done`: add `GitOp::Undo { .. }` to the `select_head` list so the graph
  jumps to the new HEAD.
- `app/plugins.rs` `op_event`: no new plugin event — `head.changed` already
  fires from the refs reload; return early for `GitOp::Undo` (and emit
  `stash.created` when `stash: true`, like checkout). Keeps the events docs
  unchanged except a line noting undo/redo surface as `head.changed`.
- Tests:
  - `tests/app.rs` (insta): `z` on a clean sandbox with two commits moves HEAD
    and shows the notice; `z` with a dirty tree opens the confirm dialog
    (snapshot); `Z` after `z` restores; "Nothing to undo" on a fresh repo.
  - `crates/tt-git/tests/e2e.rs`: one ConPTY test — commit, `z`, `Z`,
    asserting the graph's HEAD row.

## Docs (per AGENTS.md)

Paths under `docs-site/src/content/docs/`.

- New page `git/undo.mdx` ("Undo and redo"): the keys, the table of what
  each action becomes, when it asks (uncommitted changes) and what happens to
  them (popped back after a reset, left on the Stash tab after a switch), the
  refusals, the "new action clears redo" rule, a note that undoing a pushed
  commit doesn't push anything (your next push will need a force push, which
  tt-git doesn't do), and that it only follows HEAD's reflog (branch moves
  made without HEAD, e.g. `fetch`, aren't steps). Sidebar entry in
  `docs-site/astro.config.mjs` after "Merge, rebase and cherry-pick".
- `reference/keys.mdx`: two rows in "Everywhere".
- `configuration/keys.mdx`: `undo` / `redo` in the Commands table.
- `README.md` key table and `docs/configuration.md` table: `z` / `Z`.
- `reference/how-it-works.mdx`: a short "Undo and redo" section — the
  `git log -g` read, `GIT_REFLOG_ACTION` markers, `reset --hard` /
  `switch` / `switch --detach`, the stash/pop around it.
- `getting-started/navigation.mdx` / `git/palette.mdx`: mention Undo/Redo
  in the global list / palette builtins if those pages enumerate them.
- `plugins/events.mdx` + `docs/plugins.md`: one sentence under
  `head.changed` that undo/redo report through it.
- `panes/stash.mdx`: undo with uncommitted changes can leave a stash there.
- Screenshots: `palette` and `help` captures list global commands, so
  regenerate them (docshots with `CARGO_TARGET_DIR=target/agent`, then
  `pnpm screens`). Optional new capture of the undo confirm dialog in
  `docshots.rs` + `shots.mjs` for the new page.
- `cd docs-site && pnpm build` must pass.

## Order of work

1. Core parse + walker + unit tests (pure, fast to iterate).
2. `undo_step` / `apply_undo` + Sandbox tests, including the marker-shape
   test against real git.
3. TUI command, action, modal, loaders; insta tests; e2e test.
4. Docs pages, screenshots, `pnpm build`.
5. `cargo insta test --workspace --check --unreferenced reject`,
   `cargo test --workspace --doc`, clippy, fmt — all with
   `CARGO_TARGET_DIR=target/agent`, scratch sandboxes only (never this repo).

## As built (differences from the above)

- Resets use `git reset --keep`, not `--hard`: identical on a clean tree,
  but it refuses instead of overwriting anything (including an untracked
  file the target tracks).
- Checkouts are skipped as no-ops by comparing branch names, not commits,
  so switching between two branches at the same commit is still a step.
- New `GitError::Refused` for tt-git's own refusals ("HEAD moved…"), so the
  popup doesn't say "unexpected git output".
- A pop that doesn't go back cleanly returns an error starting with
  `UNDO_CHANGES_KEPT`; `op_done` shows it as-is (no "failed" prefix) and
  still selects the new HEAD.
- No insta snapshot for the dialog; the TUI test asserts its exact message.
- Review fix: a rebase of another branch (`git rebase main feat` run on
  main) is undone by switching back to where HEAD was (found from the
  newest checkout before the rebase started) and moving the rebased branch
  back with `update-ref`, its old tip read from the branch's own reflog
  (`RepoRefs::before_rebase`). Unknown old tip → switch back only, never
  move the branch. Redo does the reverse. Both check the branch hasn't
  moved before acting.
- Human review: undoing a commit discarded its changes (reset --keep moved
  the files too). Your own commits (`commit:` / `commit (amend):`) are now
  undone and redone with `reset --soft` (`UndoTarget::SoftReset`): the
  changes come back staged, nothing is ever overwritten, so there's no
  stash prompt for them. Merges, pulls, cherry-picks, reverts, resets and
  rebases still use `reset --keep`.
- Review round 2: the branch reflog wins when it shows the branch started
  at HEAD's commit (ordinary rebase), and `Branch: renamed … to <y>` in
  HEAD's reflog counts as HEAD being on `<y>`, so renaming the current
  branch before a rebase doesn't make undo detach HEAD.
- Human review 2 (2026-10-21): redo dropped ("no one really uses that").
  Gone: `Z` / `Command::Redo`, `UndoDirection`, `REDO_MARK`,
  `UndoTarget::EnterRebase`, `redo_target`; `plan` / `Repo::undo_step`
  take no direction, `NoStep::Nothing` has no payload, labels are always
  `undo …`. The counter is now just: `[tt-git undo]` adds one, each action
  passed takes one. `[tt-git redo]` entries left in reflogs by the
  pre-release build count as ordinary non-steps. The page is titled
  "Undo"; the sections above that mention redo are history.

## Open questions / risks

- `reset --hard` vs `--keep`: plan uses stash + `--hard` + pop (lazygit's
  autostash). `--keep` would avoid the stash but refuses rather than asks
  whenever a changed file differs between the commits.
- Undoing a commit that's already pushed is allowed (no remote write); the
  docs warn. Could add a confirm later if the user wants it.
- Reftable repos: `git log -g` still works; no file reads involved.
- A reflog shortened by `reflog expire` just means fewer steps.
