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
syntax = true       # color code by language in diffs, staging and blame
theme = "default"   # color theme; see Themes below
theme_background = false  # paint the theme's background instead of leaving the terminal's

[graph]
max_commits = 20000             # loaded at a time; scrolling near the end loads more
max_lanes = 16                  # lanes drawn before collapsing into …
current_includes_upstream = true  # current-branch mode also shows HEAD's upstream
dates = "relative"              # graph and blame dates: "relative" (3h, 2d) or "clock" (2026-10-17 01:30 PM, local time)
lanes = true                    # draw the lanes at startup; L in the graph toggles them
filter_to_cwd = false           # started in a subfolder, show only its commits and Status files (. toggles; --filter-to-cwd / --no-filter-to-cwd override)
# message_presets = ["^feat", "^fix"]  # regexes tab steps through in the / message filter; unset, the repo's 8 most used subject prefixes

[status]
tree = false                    # list changed files as a folder tree at startup; ` toggles it and saves it here

[branches]
unmerged_exclude = ["archive"]  # hide unmerged branches whose name contains any of these
safe_branches = ["master", "main", "production", "prod"]  # never list these on the merged/unmerged tabs
check_merges = true             # mark branches that would conflict with the base branch (git 2.40+)

[plugins]
pane_release_keys = ["ctrl+\\", "f12"]   # leave/close plugin panes

[editor]                        # for `e`; both unset by default
# command = "nvim"              # default: $VISUAL, $EDITOR, git's core.editor, then notepad / vi
# terminal = true               # runs in this terminal; default: worked out from the program

[agent]                         # for `A` in the graph, the staging view and details
command = "claude"              # the prompt is its last argument, or put {prompt} / {prompt_file} in it
placement = "auto"              # "auto" (herdr tab inside herdr, else the details pane), "herdr-tab", "herdr-split", "details"
# system_prompt = "..."         # default: tt-git's own; "" for none
# hunk_prompt = "..."           # asking about a diff hunk, after the system prompt; {file} {lines} {hunk} {source}
# [[agent.actions]]             # replaces the built-in actions: name, prompt, description, when = "always" | "merging" | "hunk"

[ui]                            # the desktop app's; the TUI ignores it
zoom = 100                      # how big everything is drawn, in percent (50–300); ctrl+= / ctrl+- / ctrl+0 in the app save it here. Global config file only
# font = "JetBrains Mono"       # the font everything is drawn in; unset: the OS's usual monospace font. Picking one in the app (Font) saves it here. Global config file only
font_size = 13                  # the font size at 100% zoom, in pixels (6–72). Global config file only
style = "native"                # "native": panes as cards, headers and dialogs in the OS's UI font; "terminal": as the TUI draws. Native/terminal style in the palette or the tt-git menu saves it here. Global config file only
check_for_updates = true        # look for a new release once a day and say so in the help bar. Global config file only
notifications = false           # a system notification (a toast on Windows) when a fetch brings new commits and the window isn't focused; on Windows also a taskbar badge for them, conflicts, or a stopped merge or rebase; on macOS a Dock badge
```

## Themes

`theme` picks the colors. Leave it out (or set `"default"`) for tt-git's own
colors in the terminal's palette. Otherwise it's one of:

- a built-in theme: `catppuccin-mocha`, `catppuccin-macchiato`,
  `catppuccin-frappe`, `catppuccin-latte`, `dracula`, `everforest`,
  `gruvbox-dark`, `gruvbox-light`, `kanagawa`, `nord`, `one-dark`,
  `rose-pine`, `rose-pine-dawn`, `solarized-dark`, `solarized-light`,
  `tokyo-night`
- `"omarchy"`: Omarchy's current theme
  (`~/.local/state/omarchy/current/theme/colors.toml`). tt-git follows it as
  you switch themes in Omarchy.
- `"omarchy:<name>"`: one of Omarchy's themes, from
  `~/.config/omarchy/themes/<name>` or `$OMARCHY_PATH/themes/<name>`
- your own: `<config dir>/themes/<name>.toml`, in Omarchy's `colors.toml`
  format (`accent`, `selection`, `muted`, `background`, `foreground`,
  `dark_foreground`, `red` … `magenta`, `bright_*`; all optional). A file
  named like a built-in replaces it.

tt-git watches the theme's file and redraws when it changes. *Theme* in the
command palette lists every theme, shows each as you move through the list,
and saves the one you pick with `enter` to `config.toml`. A theme leaves the
terminal's background alone unless `theme_background = true`.

## Merged and unmerged branches

The branches pane has four tabs, five once you have stashes; `]` and `[`
(or `}` and `{`) move between them:

- **Branches**: every local and remote branch, most recently active first.
  A branch whose merge into the base branch would conflict shows `✗2` (two
  files in conflict) after its name, and the bottom border names the
  selected one's files. The same mark shows on Unmerged and in a worktree's
  state. Turn it off with `check_merges = false`.
- **Tags**: every tag, newest first, with the tag message's subject (or the
  commit's, for a lightweight tag). The five newest also show how far they
  are from the base branch: `↓12` means the base has 12 commits since the
  tag, `↑3` that the tag has 3 the base doesn't (say, a fix tagged on a
  release branch), and `✓` that it's the base's tip. Older tags are left
  blank, since each comparison walks history and gets slower the older the
  tag. Selecting one moves the graph to its
  commit; `n` makes a branch from it, `M`/`r` merge or rebase onto it, `c`
  copies its name, `T` makes another tag at its commit, `P` pushes it and
  `d` deletes it (`y` locally, `r` locally and on the remote).
- **Unmerged**: branches on the base branch's remote that aren't merged into
  it, newest commit first, with each tip's subject and author (like
  `git branch -r --no-merged origin/main`). Names matching
  `unmerged_exclude` are left out.
- **Merged**: the ones that are.
- **Stash**: the stash list, newest first, shown once there's a stash.
  `space` pops the selected one, `a` applies it and keeps it, `d` drops it,
  `n` makes a branch from it and `enter` shows what it holds. `s` in the
  status pane stashes.

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

## Opening files in your editor

`e` in the status pane, the staging view and the details pane opens the
file in your editor, at the line you're on where tt-git can tell: the
first change (or first conflict marker) from the status pane, the
selected hunk or line in the staging view, and the line at the top of the
diff in details. A file from a commit that isn't in the working tree (on
another branch, or deleted) opens as a read-only copy of the commit's
version, written under `<temp>/tt-git/<repo>/<commit>/`; a file the commit
deleted, as it was just before. A file missing from the working tree in the
status pane is refused with a notice.

The editor is the first of `[editor] command`, `$VISUAL`, `$EDITOR` and
git's `core.editor` that is set, else `notepad` on Windows and `vi`
elsewhere. tt-git adds the file and line in the form the editor
understands: `+12 file` for vim, nvim, nano, emacs, micro and kakoune;
`--goto file:12` for VS Code, VSCodium and Cursor; `file:12` for Helix,
Zed and Sublime Text; `-n12 file` for Notepad++; `--line 12 file` for
JetBrains IDEs; just the file for anything else. Put `{file}` and
`{line}` in `command` to say it yourself:

```toml
[editor]
command = "code --reuse-window --goto {file}:{line}"
```

An editor that runs in the terminal (vim, nano, helix…) gets the whole
terminal until you quit it; then tt-git comes back and refreshes. One
with a window of its own (VS Code, Zed, Sublime, Notepad++, JetBrains
IDEs, Notepad) opens beside tt-git, which keeps running. Unknown editors
are treated as terminal ones; set `terminal = false` for a GUI editor
tt-git doesn't recognize.

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
press `space` again (in the graph or details), `esc`, or focus a pane it covers. Status,
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
the left and the selected file's diff on the right. `}` in the details pane
switches it to the same view without going fullscreen, with the files listed
below the diff.

`+` zooms whichever layout you use: the focused pane (details, or else the
graph) fills the screen, and the next `+` goes back. `space` in a fullscreen
graph opens details below it at full width and closes it again.

## Key bindings

Bindings belong to a context: `global`, `status`, `branches`, `graph`,
`details`, `stage` (the details pane while it stages a file by hunk),
`blame` (the details pane while it blames a file),
`stash` (the branches pane on its Stash tab, which doesn't fall back to
`branches`) or `worktrees` (the branches pane on its Worktrees tab, which
doesn't either).
tt-git looks up a key in the focused pane's context first, then in `global`. When you set a command's keys, they replace that command's
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
| `focus_graph`, `focus_status`, `focus_branches`, `focus_details`, `focus_next`, `focus_prev` | `1`–`4`, `tab`, `shift+tab` | global |
| `focus_left`, `focus_down`, `focus_up`, `focus_right` | `ctrl+h`, `ctrl+j`, `ctrl+k`, `ctrl+l` | global |
| `zoom` | `+` | global |
| `expand_details` | `enter` | graph |
| `toggle_details` | `space` | graph |
| `details_page_down`, `details_page_up` | `shift+pagedown`, `shift+pageup` (pages details without focusing it) | graph |
| `close_details` | `esc` (graph); `esc` `space` (details) | graph, details |
| `move_up`, `move_down`, `page_up`, `page_down`, `top`, `bottom` | `k`/`↑`, `j`/`↓`, `ctrl+u`, `ctrl+d`, `g`, `G` | global |
| `refresh`, `push`, `pull` | `ctrl+r`, `P`, `p` | global |
| `undo` | `z` | global |
| `toggle_scope`, `toggle_dates`, `toggle_lanes`, `toggle_merges`, `checkout`, `filter`, `new_branch`, `new_tag`, `merge`, `rebase`, `cherry_pick`, `revert`, `reset`, `select_range`, `agent` | `a`, `t`, `L`, `m`, `o`, `f`, `n`, `T`, `M`, `r`, `C`, `R`, `X`, `v`, `A` | graph |
| `filter_message` | `/` | graph |
| `filter_folder` | `.` | graph, status |
| `checkout`, `new_branch`, `new_tag`, `delete_branch`, `rename_branch`, `fetch`, `prune_gone`, `fast_forward`, `filter`, `merge`, `rebase`, `reset` | `enter`/`space`, `n`, `T`, `d`, `R`, `f`, `D`, `F`, `/`, `M`, `r`, `X` | branches |
| `push_tag` | `P` (on the Tags tab; pushes the branch on the others) | branches |
| `next_tab`, `prev_tab`, `pick_base` | `]` `}`, `[` `{`, `b` | branches |
| `next_tab`, `prev_tab` | `]` `}`, `[` `{` (steps the branches pane's tabs) | graph |
| `stage_toggle`, `open_staging`, `stage_all`, `discard`, `discard_all`, `ignore`, `commit`, `amend`, `stash`, `continue`, `abort` | `space`, `enter`, `a`, `d`, `D`, `i`, `c`, `A`, `s`, `C`, `X` | status |
| `toggle_tree`, `collapse_all`, `expand_all` | `` ` ``, `-`, `=` | status |
| `stash_pop`, `stash_apply`, `stash_drop`, `new_branch`, `expand_details`, `next_tab`, `prev_tab`, `filter` | `space`, `a`, `d`, `n`, `enter`, `]` `}`, `[` `{`, `/` | stash |
| `open_worktree`, `remove_worktree`, `new_branch`, `copy`, `next_tab`, `prev_tab`, `filter` | `enter`/`space`, `d`, `n`, `c`, `]` `}`, `[` `{`, `/` | worktrees |
| `stage_toggle`, `switch_side`, `toggle_line_mode`, `select_range`, `discard`, `commit`, `close_details` | `space`, `tab`, `a`, `v`, `d`, `c`, `esc` | stage |
| `next_file`, `prev_file` | `]`, `[` | stage |
| `toggle_files`, `open_link` | `}`, `o` | details |
| `older_commit`, `newer_commit` | `]`, `[` | details |
| `search`, `search_next`, `search_prev` | `/`, `n`, `N` | details |
| `open_in_editor` | `e` | status, details, stage, blame |
| `blame` | `b` | status, details |
| `agent` | `A` | details, stage (the graph's is in the row above) |
| `open_commit`, `toggle_details`, `blame_back`, `blame_forward`, `blame_older`, `blame_newer`, `search`, `search_next`, `search_prev`, `toggle_dates`, `close_details` | `enter`, `space`, `[`, `]`, `{`, `}`, `/`, `n`, `N`, `t`, `esc` | blame |
| `go_to_line` | `ctrl+g` | blame |
| `copy` (commit hash / branch name / absolute worktree path) | `c` | graph, branches, worktrees, blame |
| `plugin_logs` | (palette only) | global |
| `theme` | (palette only) | global |
| `self_update` | (palette only) | global |
| `about` | (palette only; also the desktop app's tt-git menu) | global |
| `open_repository` | (palette and menu only; desktop app only, the TUI just says it needs the desktop app) | global |
| `zoom_in`, `zoom_out`, `reset_zoom` | `ctrl+=` `ctrl++`, `ctrl+-`, `ctrl+0` (desktop app only, which also zooms with ctrl+wheel; the TUI just says they need the desktop app) | global |
| `font` | (palette and menu only; desktop app only, the TUI just says it needs the desktop app) | global |
| `window_style` | (palette and menu only; desktop app only, the TUI just says the terminal draws its own) | global |

### Plugin action bindings

```toml
[[keys.command]]
key = "ctrl+o"
type = "plugin_action"
command = "acme.github.open-commit"
context = "graph"         # optional, default: everywhere
```

## Per-repository config

A repository can override `config.toml` with files of its own, which take the
same settings:

- `.tt-git.toml` in the repository's root, to commit and share.
- `.git/tt-git.toml`, your own, not committed; every worktree reads it.

They're read in that order after `config.toml` (or `--config FILE`), and a
later file wins. Tables merge key by key; lists and `[layout]` are replaced
whole. Opening another worktree reads that worktree's files.

Either file can set any setting, `[editor]`, `[agent]`, `[plugins]` and
`[[keys.command]]` included. `tt-git config path` prints the files that apply
to a repository.

## Logs

tt-git writes its own log to `<state>/logs/tt-git.log`. `<state>` is
`$TTGIT_STATE_DIR` or the platform's state/data directory. Set the log
level with `TTGIT_LOG`, for example `TTGIT_LOG=debug` or
`TTGIT_LOG=tt_git_core=trace`.

Every running tt-git appends to the same file. When it has grown past
10 MB, the next tt-git to start renames it to `tt-git.log.1` (replacing
an older one) and starts a fresh `tt-git.log`.
