# Plan: worktree management

List, open and remove worktrees from inside tt-git. Until now it could only
create them (<kbd>n</kbd>, then <kbd>tab</kbd> to *Worktree* in the New
branch prompt).

## Behaviour

- A **Worktrees** tab in the branches pane, after Stash. Like Stash, it only
  shows when there's something to list: more than the main worktree (or
  while it's the current tab).
- Rows, main worktree first (git's order):
  `* <path>  <branch | (detached abc1234) | (bare)>  <state>`.
  - `*` marks the worktree tt-git is running in.
  - Path: the main worktree by its folder name, others relative to it
    (`.worktrees/x`, forward slashes) or in full when outside it.
  - State: `main`, `locked`, `missing` (git says *prunable*), and a change
    count (`clean`, `3 changed`, `?` if git failed, `…` until counted),
    joined with ` · `.
- Change counts aren't part of the refs load (a `git status` per worktree on
  every refs change would cost too much). They're counted in a background
  job when the tab is selected and after each refs reload while it's showing.
  One job at a time; a request during one reruns it once at the end.
- Selecting a row selects its HEAD commit in the graph, like the Branches tab.
- Keys, in a new `worktrees` context (doesn't fall back to `branches`, same
  as `stash`):

  | Key | Command | Does |
  |---|---|---|
  | <kbd>enter</kbd> <kbd>space</kbd> | `open_worktree` | Open the worktree in place of this one |
  | <kbd>d</kbd> | `remove_worktree` | Remove it (asks; asks again before `--force`) |
  | <kbd>n</kbd> | `new_branch` | New branch prompt, already on *Worktree*, from its branch (or commit) |
  | <kbd>c</kbd> | `copy` | Copy its full path |
  | <kbd>/</kbd> <kbd>]</kbd> <kbd>[</kbd> | `filter`, `next_tab`, `prev_tab` | As on the other tabs; the filter matches the path |

### Open (switch)

The app is built around one `Repo` (watcher, plugin host, socket, pane
state, in-flight jobs all hold it), so swapping it in place would leave
stale jobs reporting into the new state. Instead:

- `App::open_worktree` runs `Repo::discover` on the path (so a bad path is an
  error popup and tt-git stays put), stores it in `reopen`, and quits.
- `main` loops: after the runner returns, `take_reopen()` → drop the old app
  (its `Drop`s stop panes, the watcher, the socket and the plugin lease) →
  `set_current_dir` to the new root → build a new `App` with the same config
  and plugin options on the same terminal. Mouse capture and the alternate
  screen stay; the terminal title is set per app.
- Effectively "tt-git started there": plugins restart, `app.started` and
  `repo.opened` fire again, the socket gets a new address.
- Changing the working directory matters on Windows: a process's current
  directory can't be deleted, so staying in the old worktree would block
  removing it later.
- Refused with a notice: the current worktree, a bare repo, a missing folder,
  and anything while a git write is running (`busy`).

### Remove

- Refused with a notice, no dialog: the main worktree, the current one, and
  a locked one (git would need `-f -f`; tt-git says to `git worktree unlock`
  first). Core `remove_worktree` also refuses the main and current ones
  (`GitError::Refused`), so the UI isn't the only guard.
- Otherwise a confirm: *Remove worktree .worktrees/x? Its folder is
  deleted. Branch x stays.* plus a note that changes will be asked about.
  A missing folder gets *folder is already gone; drop it from git's list?*
  (`git worktree remove` handles a missing folder fine).
- `git worktree remove <path>`. If git refuses with "contains modified or
  untracked files", `op_done` opens a second confirm (*Uncommitted changes …
  git worktree remove --force*), like the force-delete of an unmerged
  branch. Relying on git's refusal rather than the shown count means a stale
  count can never lead to deleting changes without asking.
- New plugin event `worktree.removed` `{path, branch, force}` (added to
  `HOOK_EVENTS` so hooks on it don't warn).

### Branches pane title

Six tabs don't fit a narrow pane, and the last one (Worktrees) was the one
cut off. When the full title doesn't fit, only the current tab keeps its
count. Titles that fit are unchanged.

## Code changes

- `tt-git-core`: `model::Worktree`; `parse::parse_worktrees` (porcelain,
  unit-tested incl. locked/prunable/bare/unborn); `Repo::worktrees`,
  `Repo::worktree_changes`, `repo::same_path` (canonicalized, so git's
  `C:/x` matches `C:\x` and short names); `Repo::remove_worktree`.
- `tt-git-tui`: `BranchTab::Worktrees`; `Context::Worktrees`;
  `Command::{OpenWorktree, RemoveWorktree}`; `RefsSnapshot::worktrees`;
  actions `CountWorktreeChanges`, `WorktreeChanges`, `OpenWorktree`;
  `GitOp::RemoveWorktree { path, name, branch, force }`; `App::reopen` /
  `take_reopen`.
- `tt-git`: `main` reopens in a loop.
- `tt-git-testkit`: `TestDriver::at` opens a driver in a linked worktree;
  docshots gets a separate worktrees scene (`worktrees-tab`,
  `worktree-remove`) so the other captures don't change.

## Tests

- Core (`tests/ops.rs`): listing (current from main and from inside a
  linked worktree, locked, detached, change counts), remove clean / dirty
  (refused, then forced) / locked / main / current / missing folder.
- TUI (`tests/worktrees.rs`): snapshot of the rows; tab hidden with only the
  main worktree; open requests a reopen (and not for the current one);
  remove asks, removes, keeps the branch; dirty asks again and `n` keeps it;
  refusals; running inside a linked worktree marks it and refuses to remove
  it; `n` makes a worktree; filter; narrow title.
- e2e: open a linked worktree in the real binary and see its status title.

## Open questions

- No in-place switch without restarting plugins. If plugin authors need it,
  a `repo.closed` event before the restart would help them clean up.
- Change counts for other worktrees go stale between refs reloads (the
  watcher only watches the open one); <kbd>ctrl+r</kbd> recounts.
- Branches checked out in other worktrees aren't marked on the Branches tab
  yet; checking one out still fails with git's "already used by worktree"
  error.
