# Configuration

tt-git reads `config.toml` from the first of these locations that is set:

- `$TTGIT_CONFIG_DIR`
- `$XDG_CONFIG_HOME/tt-git`
- `~/.config/tt-git` on Linux and macOS
- `%APPDATA%\tt-git` on Windows

You can also pass a file with `--config FILE`. Every setting is optional.
Unknown keys are reported as errors, so typos don't go unnoticed.

```toml
watch = true        # refresh when files or refs change outside tt-git
mouse = true        # click, wheel, ctrl+click links

[graph]
max_commits = 20000             # loaded at a time; scrolling near the end loads more
max_lanes = 16                  # lanes drawn before collapsing into …
current_includes_upstream = true  # current-branch mode also shows HEAD's upstream

[branches]
unmerged_exclude = ["archive"]  # hide unmerged branches whose name contains any of these
safe_branches = ["master", "main", "production", "prod"]  # never list these on the merged/unmerged tabs

[plugins]
pane_release_keys = ["ctrl+\\", "f12"]   # leave/close plugin panes
```

## Merged and unmerged branches

The branches pane has three tabs; `]` and `[` (or `}` and `{`) move
between them:

- **Branches**: every local and remote branch, most recently active first.
- **Unmerged**: branches on the base branch's remote that aren't merged into
  it, newest commit first, with each tip's subject and author (like
  `git branch -r --no-merged origin/main`). Names matching
  `unmerged_exclude` are left out.
- **Merged**: the ones that are.

Neither list includes the base branch itself or the long-lived branches in
`safe_branches` (by default `master`, `main`, `production` and `prod`),
matched by exact branch name on any remote. Set it to `[]` to list them
too.

On the Unmerged and Merged tabs the graph shows only the selected branch:
its own commits (first-parent, so commits it pulled in from other branches
are left out), the base commit it branched off, and the base commit that
merged it in. For an unmerged branch, the base's current tip takes the
merge's place, so you can see how far the base has moved on. The base's
commits in between are skipped. A branch that was fast-forwarded into the
base has no commits of its own left, so only its tip is shown. Going back
to the Branches tab, or pressing `a` in the graph, shows the whole graph
again.

The base branch is the project's primary branch: whatever `origin/HEAD`
points at, else the first of `main`, `master`, `development`, `develop` or
`trunk` found on a remote (then locally). Press `b` on the branches pane
to pick a different one from a list; tt-git saves it in the repo's own git
config as `tt-git.baseBranch`, so each project remembers its own.
"Detect automatically" at the top of the list clears it again.

## Layout

`[layout]` is a tree. Each node is either a pane (`status`, `branches`,
`graph` or `details`) or a split whose `children` sit in `rows` (top to
bottom) or `columns` (left to right). `size` is a percentage of the parent
split; children without a size share whatever is left. This is the default:

```toml
[layout]
split = "rows"
children = [
  { pane = "graph", size = 70 },
  { split = "columns", details = true, children = [{ pane = "status" }, { pane = "branches" }] },
]
```

The graph spans the top 70%, with status and branches side by side below it.
`details = true` marks where the details pane opens: press `space` on a
commit in the graph (or `4`) and details takes that node's place until you
press `space` again, `esc` (in the graph or details), or focus a pane it covers. Status,
branches and graph must each appear once.

To keep details on screen all the time, give it a pane of its own instead.
This is the earlier four-pane layout:

```toml
[layout]
split = "columns"
children = [
  { size = 33, split = "rows", children = [{ pane = "status", size = 40 }, { pane = "branches" }] },
  { split = "rows", children = [{ pane = "graph", size = 65 }, { pane = "details" }] },
]
```

`enter` on a commit opens it fullscreen instead, with the changed files on
the left and the selected file's diff on the right.

`+` zooms whichever layout you use: first to just the graph and details at
full width (the graph alone, full height, if details is closed; `space` still
toggles details), then to the focused pane alone, then back.

## Key bindings

Bindings belong to a context: `global`, `status`, `branches`, `graph` or
`details`. tt-git looks up a key in the focused pane's context first, then
in `global`. When you set a command's keys, they replace that command's
default keys in that context.

```toml
[keys.global]
quit = ["q", "ctrl+c"]
palette = ":"

[keys.graph]
toggle_scope = "b"

[keys.branches]
checkout = ["enter", "o"]
```

Write keys as `q`, `Q`, `shift+q`, `ctrl+c`, `alt+enter`, `shift+tab`,
`space`, `f5`, `pageup`, `home`, `+` or `ctrl++`.

| Command | Default | Context |
|---|---|---|
| `quit`, `help`, `palette` | `q` `ctrl+c`, `?`, `:` `ctrl+p` | global |
| `focus_status` … `focus_details`, `focus_next`, `focus_prev` | `1`–`4`, `tab`, `shift+tab` | global |
| `focus_left`, `focus_down`, `focus_up`, `focus_right` | `ctrl+h`, `ctrl+j`, `ctrl+k`, `ctrl+l` | global |
| `zoom` | `+` | global |
| `expand_details` | `enter` | graph |
| `toggle_details` | `space` | graph |
| `close_details` | `esc` | graph, details |
| `move_up`, `move_down`, `page_up`, `page_down`, `top`, `bottom` | `k`/`↑`, `j`/`↓`, `ctrl+u`, `ctrl+d`, `g`, `G` | global |
| `refresh`, `push`, `pull` | `ctrl+r`, `P`, `p` | global |
| `toggle_scope`, `filter`, `new_branch`, `merge`, `rebase`, `cherry_pick` | `a`, `f`, `n`, `M`, `r`, `C` | graph |
| `checkout`, `new_branch`, `delete_branch`, `rename_branch`, `fetch`, `filter`, `merge`, `rebase` | `enter`/`space`, `n`, `d`, `R`, `f`, `/`, `M`, `r` | branches |
| `next_tab`, `prev_tab`, `pick_base` | `]` `}`, `[` `{`, `b` | branches |
| `stage_toggle`, `stage_all`, `discard`, `commit`, `amend`, `continue`, `abort` | `space`/`enter`, `a`, `d`, `c`, `A`, `C`, `X` | status |
| `open_link` | `o` | details |
| `copy` (commit hash / branch name) | `c` | graph, branches |
| `plugin_logs` | (palette only) | global |

### Plugin action bindings

```toml
[[keys.command]]
key = "ctrl+o"
type = "plugin_action"
command = "acme.github.open-commit"
context = "graph"         # optional, default: everywhere
```

## Logs

tt-git writes its own log to `<state>/logs/tt-git.log`. `<state>` is
`$TTGIT_STATE_DIR` or the platform's state/data directory. Set the log
level with `TTGIT_LOG`, for example `TTGIT_LOG=debug` or
`TTGIT_LOG=tt_git_core=trace`.
