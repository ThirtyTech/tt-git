# Plan: stash individual files, or only staged / only unstaged changes

Notion task "Stash individual files": stash only the selected file, or only
the staged or only the unstaged changes, from the status pane.

## What was already there

<kbd>s</kbd> in the status pane opens the **Stash** prompt, with a
<kbd>tab</kbd>-cycled scope (`StashScope` in tt-git-core):

- **Everything**: `git stash push --include-untracked`
- **Staged**: `git stash push --staged` (git 2.35)
- **Keep staged**: `git stash push --include-untracked --keep-index`
- *the selected file*: `git stash push --include-untracked -- <path> [<orig>]`

Missing or wrong:

- No "only the unstaged changes". **Keep staged** isn't it: it stashes the
  staged changes too, so popping it brings them back a second time.
- The selected file was passed as a pathspec, so `a[b].txt` also stashed
  `ab.txt`.
- A staged rename (both paths) or a staged deletion failed: git saved a
  stash, then failed `git add -- <old path>` and left everything in place,
  so the user got an error and a stray duplicate stash.
- **Staged** with overlapping staged and unstaged changes in one file failed
  the same way ("Cannot remove worktree changes") and left a stray stash.
- git older than 2.35 showed git's usage text for `--staged`.

The status pane has no multi-select and lists files, not directories
(`--untracked-files=all`), so "the selected file" is one entry; multi-file
selection is out of scope.

## Behaviour

- New choice **Unstaged** between **Staged** and **Keep staged**: stashes
  unstaged and untracked changes only; staged changes stay staged and aren't
  in the stash.
  - Nothing unstaged: refuses, `no unstaged changes to stash`, no git write.
  - Nothing staged: same as **Everything**.
  - Otherwise: `git stash push --staged -m "tt-git: staged changes set
    aside"`, `git stash push --include-untracked [-m msg]`, then `git stash
    pop --index` on the first stash (checked by oid, so an older stash is
    never popped). If that pop fails, the error says the staged changes are
    on the Stash tab under that message.
- **Staged** / **Unstaged** with overlapping changes: drop the stash git left
  behind (only when `stash@{0}` is new) and refuse with "the staged and
  unstaged changes overlap…". git < 2.35: "needs git 2.35 or newer".
- The selected file: paths go to git as `:(literal)<path>`. Staged deletions
  among them (`git diff --cached --no-renames --diff-filter=D`) are unstaged
  first with `git restore --staged`; popping brings them back as unstaged
  deletions.
- The stash prompt is 72 columns wide (was 60) so the row of choices fits.

## Tests

- core `tests/ops.rs` (`stash` module): unstaged only, nothing staged,
  nothing unstaged, overlap for both scopes, literal path, staged rename,
  staged deletion.
- tui `tests/stash.rs`: <kbd>tab</kbd> to **Unstaged** and stash; the
  selected file as a staged rename.

## Docs

`panes/status.mdx`, `panes/stash.mdx`, `reference/how-it-works.mdx`, README
key table; `status-stash` screenshot regenerated (shows **Unstaged**).
