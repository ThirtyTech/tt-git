# Plugins

tt-git plugins use the same model as
[herdr's plugins](https://herdr.dev/docs/plugins/). A plugin is a directory
with a `tt-git-plugin.toml` manifest and any programs it lists. There is no
SDK, embedded runtime or sandbox. tt-git starts plugin programs as normal
processes, and **the tt-git CLI is the plugin API**: a plugin runs
`$TTGIT_BIN_PATH <command>`, or talks to the JSON socket at
`$TTGIT_SOCKET_PATH` directly.

> Plugins run with your user's permissions and are not sandboxed. Install
> only code you trust. `tt-git plugin install` shows the source, the commit
> and every command it will run, then asks before doing anything.

## Manifest

```toml
id = "acme.github"               # required: ASCII letters, digits, . : _ -  (max 120)
name = "GitHub helpers"          # required
version = "0.1.0"                # required (semver; a warning otherwise)
min_tt_git_version = "0.1.0"     # required; refused if newer than this tt-git
description = "…"                # optional
platforms = ["linux", "macos", "windows"]   # optional; missing gives a warning

[[build]]                        # runs only on `plugin install` / `update`
command = ["npm", "ci"]
platforms = ["linux", "macos"]   # any item can narrow platforms

[[startup]]                      # runs once each time tt-git starts
command = ["node", "dist/restore.js"]

[[actions]]
id = "open-commit"               # local id: no dots (qualified as acme.github.open-commit)
title = "Open commit on GitHub"
description = "…"
contexts = ["commit"]            # global | commit | branch | file | selection (default: global)
command = ["node", "open.mjs", "commit"]

[[events]]
on = "push.completed"
command = ["node", "notify.mjs"]

[[panes]]
id = "board"
title = "Project board"
placement = "popup"              # overlay (default) | popup | split | tab | zoomed
width = "80%"                    # popup only: cells or "N%"
height = 20
command = ["board"]

[[link_handlers]]
id = "issue"
title = "Open GitHub issue"
pattern = '^https://github\.com/[^/]+/[^/]+/issues/[0-9]+$'   # Rust regex
action = "open-commit"           # an action in this plugin
```

Commands are argument arrays and never go through a shell. A relative
program path that contains a `/` is resolved against the plugin directory.
On Windows a bare program name is looked up on `PATH` using `PATHEXT`, so
`npm` finds `npm.cmd`. Commands run with the plugin directory as their
working directory.

If an action, pane or link handler id appears twice, or a link handler
names an action that doesn't exist, the manifest is rejected. An unknown
event name only produces a warning. Error codes match herdr's where one
exists: `invalid_plugin_id`, `plugin_requires_newer_tt_git`,
`duplicate_plugin_action_id`, and so on.

## Environment

Every plugin process gets these variables:

| Variable | |
|---|---|
| `TTGIT_ENV` | `1` |
| `TTGIT_BIN_PATH` | the running `tt-git` executable |
| `TTGIT_SOCKET_PATH` | this instance's API socket |
| `TTGIT_PLUGIN_ID`, `TTGIT_PLUGIN_ROOT` | the plugin's id and directory |
| `TTGIT_PLUGIN_CONFIG_DIR` | for files the user edits (e.g. a `.env`); created, never touched |
| `TTGIT_PLUGIN_STATE_DIR` | for the plugin's own state; created, never touched |
| `TTGIT_REPO_PATH` | the repository's working tree |
| `TTGIT_PLUGIN_CONTEXT_JSON` | what the user was looking at (below) |

Some entry points add more:

| Entry point | Adds |
|---|---|
| action | `TTGIT_PLUGIN_ACTION_ID` (qualified) |
| event hook | `TTGIT_PLUGIN_EVENT`, and `TTGIT_PLUGIN_EVENT_JSON` set to `{"event": …, "data": …}` |
| startup command | `TTGIT_PLUGIN_EVENT=startup` |
| pane | `TTGIT_PLUGIN_ENTRYPOINT_ID` |
| link handler | `TTGIT_PLUGIN_CLICKED_URL`, `TTGIT_PLUGIN_LINK_HANDLER_ID` |

tt-git sets its own variables last, so they win over any variable of the
same name.

The context JSON has these fields: `repo_path`, `repo_name`, `head`,
`branch`, `upstream`, `selected_commit`, `selected_branch`, `selected_file`,
`selected_text`, `focused_pane`, `invocation_source` (`keybinding`,
`palette`, `socket`, `event`, `startup`, `link_click`, `pane`),
`correlation_id`, `clicked_url` and `link_handler_id`.

Each command is a fresh process. Up to 32 can run at once; beyond that you
get `plugin_command_limit_reached`. tt-git keeps up to 64 KiB of stdout and
of stderr per command, then adds a truncation note. The last 200 runs are
kept in the plugin log (`tt-git plugin log list`, or *Plugin logs* in the
palette). A failed command shows a ✗ notice in the help bar.

## Running actions

You can run an action in four ways:

- From the command palette (`:` or `ctrl+p`). It lists actions whose
  `contexts` include `global` or the focused pane's context: `commit` in the
  graph, `branch` in the branch list, `file` in the status pane.
- With a key binding in `config.toml`:

  ```toml
  [[keys.command]]
  key = "ctrl+o"
  type = "plugin_action"
  command = "acme.github.open-commit"
  description = "open on GitHub"   # optional
  context = "graph"                # optional; default is everywhere
  ```

- With `tt-git plugin action invoke acme.github.open-commit`, or the socket
  method `plugin.action.invoke`.
- Through a link handler: press `o` in the details pane, or `ctrl`+click a
  link. The first handler whose pattern matches the URL runs.

## Events

Event hooks are low-volume events, each run as a fresh process:

| Event | `data` |
|---|---|
| `app.started` | `{version}` (also after opening another worktree, which starts tt-git over there) |
| `repo.opened` | `{path, name}` |
| `head.changed` | `{from, from_branch, to, branch}` (also on switching between branches at the same commit, and on undo, which has no event of its own) |
| `branch.checked_out` | `{branch, created, tracking?}`; `created` is true for auto-created tracking branches |
| `branch.created` / `branch.deleted` / `branch.renamed` | `{branch, start, worktree}` (`worktree` is the new worktree's path, or null) / `{branch, remote}` / `{from, to}` |
| `tag.created` / `tag.pushed` / `tag.deleted` | `{tag, target, annotated}` / `{tag}` / `{tag, remote}` (`remote` is null for a local-only delete) |
| `commit.created` | `{oid, amend, summary}` |
| `fetch.completed` / `pull.completed` / `push.completed` | `{}` / `{branch, head}` / `{branch}` |
| `files.staged` / `files.unstaged` / `files.discarded` | `{paths}` (`"all"` for stage-all) / `{paths}` / `{path}`, plus `partial: true` when only some hunks or lines changed; discard all sends `{path: "all", scope}` with `scope` one of `everything`, `unstaged`, `unstaged_and_untracked`, `untracked` |
| `merge.completed` / `rebase.completed` / `cherry_pick.completed` | `{target/onto/picked, head}` |
| `stash.created` / `stash.applied` / `stash.dropped` | `{message}` / `{stash, pop}` / `{stash}` |
| `worktree.removed` | `{path, branch, force}` (`branch` null if it was detached; `force` true if removed with its changes) |

A long-running process can call `events.subscribe` instead (see below) to
get a stream of events.

## Panes

A pane runs a plugin program in a pseudo-terminal inside tt-git (ConPTY on
Windows). The program draws its own terminal UI and tt-git displays it.

- **`overlay`** covers the whole screen. **`popup`** is a centred modal box
  that uses `width`/`height`; only one popup can be open at a time, and a
  second gives `ui_busy`. While either is open it gets all keystrokes.
  Pressing `ctrl+\` or `F12` closes it, and it closes by itself when the
  program exits.
- **`split`** takes the place of the details pane (`4`). Keys go to the
  program while that pane has focus. `ctrl+\` or `F12` gives focus back to
  tt-git and leaves the program running. When the program exits, the pane
  shows its exit status; any key closes it.
- tt-git has no tabs or zoom, so **`tab`** and **`zoomed`** open as overlays.

You can open a pane from the palette (*Open <title>*), with
`tt-git plugin pane open --plugin ID --entrypoint ID [--placement …]
[--width …] [--height …] [--env K=V]… [--no-focus]`, or with the socket
method `plugin.pane.open`. The keys that leave or close a pane are set by
`[plugins] pane_release_keys`.

## The CLI

| Command | |
|---|---|
| `tt-git plugin install owner/repo[/subdir…] [--ref REF] [--yes]` | install from GitHub |
| `tt-git plugin update [ID or owner/repo …] [--yes]` | update GitHub installs (skips unchanged ones and linked plugins) |
| `tt-git plugin uninstall ID-or-owner/repo` | |
| `tt-git plugin link PATH [--disabled]` / `unlink ID` | local development; no build step |
| `tt-git plugin list [--json] [--plugin ID]` | |
| `tt-git plugin enable ID` / `disable ID` / `config-dir ID` | |
| `tt-git plugin action list` / `action invoke ACTION [--plugin ID]` | needs a running tt-git |
| `tt-git plugin log list [--plugin ID] [--limit N]` | needs a running tt-git |
| `tt-git plugin pane open …` / `pane focus PANE` / `pane close PANE` | needs a running tt-git |
| `tt-git ui notify MSG` / `ui error MSG` / `ui refresh` / `ui focus PANE` | needs a running tt-git |
| `tt-git events [--event NAME]…` | stream events as JSON lines |
| `tt-git status [--json]` / `tt-git branch list [--json]` | read the repo directly (uses `TTGIT_REPO_PATH`) |
| `tt-git branch decorate [--plugin ID] [--clear] [BRANCH=TEXT]…` | set this plugin's branch list column (`--plugin` defaults to `$TTGIT_PLUGIN_ID`); needs a running tt-git |
| `tt-git api METHOD [PARAMS_JSON]` | any socket method |

Commands marked "needs a running tt-git" connect to `$TTGIT_SOCKET_PATH`,
which plugins have set automatically, or to `--socket ADDRESS`.

## Socket API

The socket carries newline-delimited JSON over a Unix domain socket, or a
named pipe on Windows:

```text
→ {"id":"req_1","method":"ui.notify","params":{"message":"hi"}}
← {"id":"req_1","result":{}}
← {"id":"req_2","error":{"code":"invalid_params","message":"message is required"}}
```

| Method | Params | Result |
|---|---|---|
| `ping` | | `{pong, version}` |
| `repo.info` | | `{path, name, head, branch, upstream, ahead, behind}` |
| `status.get` | | branch headers, entries, in-progress operation |
| `branch.list` | | the unified branch list, with `state` and `decorations` (`{plugin_id: text}`) |
| `branch.decorations.set` | `{plugin_id, decorations: {branch: text}}` | `{count}`; replaces the plugin's branch column (below) |
| `graph.selection` | | `{scope, branch, index, commit}`; `branch` is set while the graph shows one branch from the merged/unmerged tabs |
| `ui.notify` / `ui.error` | `{message}` | |
| `ui.refresh` | | |
| `ui.focus` | `{pane}`: status, branches, graph, details | |
| `plugin.list` | | installed plugins |
| `plugin.link` | `{path, enabled?}` | |
| `plugin.unlink` / `plugin.enable` / `plugin.disable` | `{plugin_id}` | |
| `plugin.action.list` | `{plugin_id?}` | actions |
| `plugin.action.invoke` | `{action_id, plugin_id?}` | `{log_id}` |
| `plugin.log.list` | `{plugin_id?, limit?}` | command logs |
| `plugin.pane.open` | `{plugin_id, entrypoint, placement?, width?, height?, env?, focus?}` | `{pane_id}` |
| `plugin.pane.focus` / `plugin.pane.close` / `plugin.pane.screen` | `{pane_id}` | `{}` / `{}` / `{text}` |
| `plugin.pane.list` | | open panes |
| `popup.close` | | `{closed}` |
| `events.subscribe` | `{events?: [names]}` | `{subscribed}`, then one event envelope per line |

## Branch columns

`branch.decorations.set` (or `tt-git branch decorate`) puts short text next
to branches in the full branch list, such as a PR number. Each call replaces
everything that plugin set; `{}` removes its column. Every plugin gets one
column between the name and the status, as wide as its widest value (at most
10 cells), so values line up. Text is matched against `name` in
`branch.list`, control characters are dropped, and text is cut to 10 cells.
The plugin must be enabled (`plugin_not_found` otherwise); disabling or
removing it clears the column. Columns aren't saved across restarts.

## Lifecycle and files

1. **Install or link.** The manifest is validated and the plugin is recorded
   in `<config>/plugins.json`. A file lock keeps concurrent commands safe.
   Its config and state directories are created.
2. **Build.** `[[build]]` runs only for GitHub installs and updates, after
   you confirm. It runs before the plugin is registered and gets none of the
   runtime `TTGIT_*` variables. If a build step fails, or changes the
   manifest, nothing is installed.
3. **Startup.** `[[startup]]` runs each time tt-git starts, once the socket
   is ready, followed by the `app.started` and `repo.opened` events.
4. **Actions, events and panes** run on demand. A running tt-git picks up
   registry changes from `tt-git plugin …` commands within a second.

Files live under `<config>`, which is `$TTGIT_CONFIG_DIR`, else
`$XDG_CONFIG_HOME/tt-git`, `~/.config/tt-git` or `%APPDATA%\tt-git`:

- `plugins.json` lists installed plugins and their sources.
- `plugins/github-installations/<id>/<pid>-<generation>/checkout` holds
  managed checkouts. Every install or update creates a new generation, so
  plugin processes that are already running keep their files. Old
  generations are deleted after an install, update or tt-git start, once no
  running tt-git holds a lease on them.
- `plugins/config/<id>/` is the plugin's config directory.

Plugin state lives in `<state>/plugins/<id>/`.

## Testing a plugin

Run `tt-git plugin link .` from your plugin directory, start `tt-git`, and
open *Plugin logs* from the palette to see every run's exit code, stdout
and stderr. `tt-git events` shows the events your hooks would receive.
