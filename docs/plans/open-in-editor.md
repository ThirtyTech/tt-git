# Plan: open the selected file in your editor

Press <kbd>e</kbd> on a file in the status pane, the staging view or the
details pane and tt-git opens it in your editor, at the line you're on where it
can work one out. A terminal editor (vim, nvim, nano, helix…) takes over the
terminal and tt-git comes back when you quit it. A GUI editor (VS Code, Zed,
Sublime…) opens beside it and tt-git keeps running.

## Behaviour

### The key

- New command `open_in_editor`, label "edit", default <kbd>e</kbd> in the
  `status`, `stage` and `details` contexts. `e` is free in all three and in
  `global`. Not bound in graph/branches/stash.
- Shown in the help bar of those three views (add to each `commands()` list)
  and in the palette/help popup like any bound command.

### Which file, which line

| Where | File | Line |
|---|---|---|
| Status pane | the selected entry's `path` (the new path for a rename) | the first changed line, if the details pane is showing this file's diff (`DetailsSubject::File(path)`, loaded); otherwise none |
| Staging view, unstaged side | the view's `path()` | hunk mode: the selected hunk's first changed line; line mode: the cursor line. A `-` line maps to the new-side line where it was removed |
| Staging view, staged side | same | as above, then mapped from index line numbers to working-tree line numbers through the unstaged diff's hunks (shift by `new_count - old_count` for every unstaged hunk that ends before the line) |
| Details, files view (`CommitFiles`) | the selected `CommitFile.path` | from the top row of the diff viewport (see below) |
| Details, file diff (`File`) | the subject's path | from the top row of the viewport |
| Details, commit / stash view (`Commit`, `Stash`) | the file whose `diff --git` section holds the top row of the viewport (or the first file below it, when the top row is in the commit header), parsed from `+++ b/<path>` | from the top row |
| Details, anything else (`Text`, nothing selected) | none: notice "no file to open here" | |

**Row → line** (pure function, unit-tested): from the row, walk up to the
nearest `@@ -a,b +c,d @@` in the same file section and count down from `c`:
context and `+` rows advance the new-side counter, `-` rows don't. A row on a
`@@` heading or a file header, or above the first hunk, uses the first changed
line of the next hunk. Lines come from `DetailsPane::lines()` with
`cursor.offset` as the top row (the details pane scrolls; it has no line
cursor). Tabs are already expanded in those lines, which doesn't affect the
prefixes this looks at.

`+++ b/…` paths can be C-quoted (`"b/\303\244.txt"`, git's default
`core.quotePath`). Unquote them (octal escapes, `\\ \" \t \n`); add a small
`unquote_c_path` to `tt-git-core::parse` unless one already exists there.
`+++ /dev/null` means the file was deleted in that commit: notice "<path> was
deleted in this commit".

Line numbers from a commit's diff are that commit's numbers. If the file has
changed since, the editor lands near rather than on the line. That's fine and
the docs say so.

### Refusals (a notice in the help bar, no editor)

- No selection / nothing loaded: "no file to open here".
- The file isn't in the working tree (deleted, or only in an old commit):
  "<path> isn't in the working tree". Check `repo_root.join(path).exists()`
  before launching. A deleted status entry hits this.
- The editor can't be started: error popup "couldn't start `<program>`:
  <io error>. Set `[editor] command` in config.toml." Same for an empty
  command.
- A terminal editor exits non-zero: notice "editor exited with status N".

After any editor closes (terminal), or right after launching (GUI), send
`Action::Refresh` so status and diffs pick up edits even with `watch = false`.

### Which editor

Resolved each time <kbd>e</kbd> is pressed, first match wins:

1. `[editor] command` in config.toml
2. `$VISUAL`
3. `$EDITOR`
4. git's `core.editor` (`git config --get core.editor`, run in the repo so
   local/global/system all apply; this is what Git for Windows' installer sets)
5. `notepad` on Windows, `vi` elsewhere

The command string is split into words (double and single quotes group;
backslash escapes only outside Windows so `C:\Program Files\…` survives), then
the program is resolved with `tt_git_plugin::runtime::resolve_program` so
`code` finds `code.cmd` through `PATHEXT` on Windows.

**Arguments.** Placeholders are substituted per word after splitting, so paths
with spaces need no quoting:

- If any word contains `{file}`, the command is a template: replace `{file}`
  with the absolute path and `{line}` with the line (1 when there isn't one).
  Nothing else is appended.
- Otherwise append the file, with the line in the form the editor understands,
  picked by the program's lowercased file stem (`.exe`/`.cmd` stripped):

  | Editors | Appended |
  |---|---|
  | `vi`, `vim`, `nvim`, `gvim`, `nano`, `micro`, `emacs`, `emacsclient`, `kak`, `joe`, `mg`, `ne` | `+<line> <file>` |
  | `code`, `code-insiders`, `codium`, `cursor`, `windsurf` | `--goto <file>:<line>` |
  | `hx`, `helix`, `zed`, `subl`, `sublime_text` | `<file>:<line>` |
  | `notepad++` | `-n<line> <file>` |
  | IntelliJ family (`idea`, `idea64`, `pycharm`, `goland`, `rustrover`, `webstorm`, `clion`, `rider`) | `--line <line> <file>` |
  | anything else (incl. `notepad`) | `<file>` |

  Without a line, just `<file>`.

The file is `repo_root` joined with the path's `/`-separated components (not
`Path::join` on the raw string, which leaves mixed separators on Windows). The
child's cwd is the repo root.

**Terminal or GUI.** `[editor] terminal = true|false` decides; unset, it's
GUI for `code`/`code-insiders`/`codium`/`cursor`/`windsurf`/`zed`/`subl`/
`sublime_text`/`notepad`/`notepad++`/`gedit`/`kate`/`mate` and the IntelliJ
family, terminal for everything else. Unknown means terminal because that's
the safe failure: a GUI editor run as "terminal" just makes tt-git wait
(or flicker if it returns at once); a terminal editor run as "GUI" would fight
tt-git for the screen.

### Config

```toml
[editor]
# Default: $VISUAL, $EDITOR, git's core.editor, then notepad / vi.
command = "code --goto {file}:{line}"
# Default: worked out from the program name.
terminal = false
```

`EditorConfig { command: Option<String>, terminal: Option<bool> }`, `#[serde(default,
deny_unknown_fields)]`, new `editor` field on `Config`.

## Design

### Where the code goes

- `crates/tt-git-core/src/patch.rs`: pure line-number helpers.
  - `Hunk::new_line_of(idx) -> u32`: new-side line for line `idx` of the hunk
    (`-` lines → the next new-side line).
  - `FileDiff::first_change_line() -> Option<u32>`.
  - `FileDiff::old_to_new(line) -> u32`: map an old-side line through the
    hunks (used for staged → working tree).
  - `diff_text_position(lines: &[String], row) -> Option<(Option<String>, u32)>`:
    the row → (file from `+++`, line) walker for raw diff text, with
    `unquote_c_path`.
- `crates/tt-git-tui/src/editor.rs` (new): `EditorConfig`, `resolve(&cfg,
  &Repo) -> Result<EditorCmd, String>`, `split_command`, `editor_kind(stem)`,
  `build_argv(cmd, file, line) -> Vec<String>`, `is_terminal(..)`. All pure
  except the `core.editor` lookup and env reads (pass env in for tests).
- `crates/tt-git-tui/src/keymap.rs`: `Command::OpenInEditor` (label "edit"),
  three `DEFAULTS` entries.
- Panes: each works out a target and emits `Action::OpenInEditor(EditTarget {
  path, line })`; they don't know about editors.
  - `StatusPane::on_command`: path from `selected()`; line is filled in by
    `App` (it owns `details`, so it can check for a loaded `File(path)` diff).
    Simplest: `App` handles `Command::OpenInEditor if focus == Status` in its
    global dispatch (next to `OpenStaging`) and builds the target there.
  - `StageView::on_command`: `Command::OpenInEditor` → target from side/hunk/
    cursor using the core helpers.
  - `DetailsPane::on_command`: files view / text views → target via
    `diff_text_position` on `lines` from `cursor.offset`.
- `crates/tt-git-tui/src/action.rs`: `Action::OpenInEditor(EditTarget)` and
  `Action::EditorClosed(Result<Option<i32>, String>)` (exit code or spawn error).
- `crates/tt-git-tui/src/app.rs`: on `OpenInEditor`, check the file exists,
  resolve the editor, build an `EditorLaunch { argv, cwd, terminal }` and store
  it in `pending_editor` (like `clipboard`). `pub fn take_editor(&mut self) ->
  Option<EditorLaunch>`. **`App` never spawns a process**, so `TestDriver`
  tests can press <kbd>e</kbd> and inspect the launch with no side effects.
  On `EditorClosed`: notice/error as above, then `Refresh`.
- `crates/tt-git-tui/src/runner.rs`: after the queued actions, `if let
  Some(launch) = app.take_editor()`:
  - GUI: `std::process::Command` with null stdin/stdout/stderr, cwd; spawn,
    don't wait; reap on a detached thread (no zombies on Unix). Spawn error →
    `Action::EditorClosed(Err)`. Success → `Refresh`.
  - Terminal: a `suspend_for(terminal, &mut events, launch, app.wants_mouse())`
    helper:
    1. drop the `EventStream` (crossterm's stream thread is woken and stops
       on drop, so it no longer reads stdin);
    2. `DisableMouseCapture` if mouse, `LeaveAlternateScreen`, show cursor,
       `disable_raw_mode`;
    3. run the child with inherited stdio, waiting via
       `tokio::task::spawn_blocking(..).await` (current-thread runtime: jobs
       and watcher results just queue in `rx` meanwhile);
    4. `enable_raw_mode`, `EnterAlternateScreen`, `EnableMouseCapture` if
       mouse, re-set the terminal title (expose `App::terminal_title()` or pass
       the title main.rs built into `run`), `terminal.clear()` so the next draw
       is full;
    5. new `EventStream`, `ticker.reset()` (avoid a burst of missed ticks),
       `app.update(Action::EditorClosed(..))`, force a redraw.
    Restore runs even if spawning failed.

### Why suspend instead of a PTY overlay pane

`PtyPane` could host a terminal editor without touching the runner, but it
draws through a vt100 emulator with no scrollback, mouse or true-colour
passthrough, and `PaneSpec`/`plugin_env` are plugin-shaped. Handing the real
terminal over is what lazygit and gitui do, and editors behave exactly as
they do outside tt-git. The runner change is contained (one helper).

## Tests

- `tt-git-core` unit tests (patch.rs): `new_line_of` on context/add/del lines;
  `first_change_line`; `old_to_new` with hunks before/after/around the line;
  `diff_text_position` on a `git show` text (row in header → first hunk of the
  first file, row in second file, `-` row, `/dev/null`, quoted path).
- `editor.rs` unit tests: resolution order with injected env and config;
  `split_command` (quotes, Windows backslashes, empty); argv per editor
  family, template with/without `{line}`, no-line case; terminal
  classification; `.exe`/`.cmd` stems.
- `crates/tt-git-tui/tests/editor.rs` (new, `TestDriver` + `Sandbox`, config
  `[editor] command = "ed {file} {line}"`, `terminal = true`) asserting
  `app_mut().take_editor()`:
  - status: <kbd>e</kbd> on a modified file → its absolute path; after the
    details pane loaded its diff → first changed line;
  - staging view: hunk mode on the second hunk → its first change; line mode
    after <kbd>j</kbd>; staged side mapped through an unstaged hunk above it;
  - details files view of a commit, scrolled → the viewport's line;
  - a deleted file → notice, no launch; `Text` subject → notice;
  - `EditorClosed(Ok(Some(1)))` → notice; `EditorClosed(Err)` → error popup.
- `crates/tt-git/tests/e2e.rs`: one ConPTY test of the real suspend/restore:
  config `[editor] command = '"<bin>" __test-plugin record <out> interactive editing {file}:{line}'`,
  `terminal = true`. Press <kbd>e</kbd> in status, wait for "editing" on
  screen, type `q` + enter, wait for the status pane again, assert the recorded
  args hold the file and line and that keys work afterwards (proves stdin is
  back with tt-git). Watch it on Windows specifically; that's where
  EventStream/console-mode handoff can go wrong.
- `tt-git-tui/tests/docs.rs` already parses every TOML block in
  `docs/configuration.md`, so the new `[editor]` example is checked.
- Snapshots: help bars for status, staging and details gain "e edit". Review
  the `.snap.new` files and accept only those.

## Docs (per AGENTS.md)

Under `docs-site/src/content/docs/`:

- `panes/status.mdx`: new section "Opening a file in your editor" (what
  <kbd>e</kbd> does, the line rule, the not-in-working-tree notice), key in its
  table and `## Keys`. In "Merges, rebases and cherry-picks in progress", step
  "Fix the conflicts in your editor" gains "(<kbd>e</kbd> opens the selected
  file)".
- `panes/staging.mdx`: section on <kbd>e</kbd> (selected hunk / line; staged
  side mapped to the working-tree line), `## Keys`.
- `panes/details.mdx`: section on <kbd>e</kbd> (file at the top of the diff
  view; commit line numbers may have moved since), `## Keys`.
- `reference/keys.mdx`: <kbd>e</kbd> under Status, Details and Staging.
- `configuration/keys.mdx`: Commands table row `open_in_editor` | <kbd>e</kbd> |
  status, details, stage.
- `configuration/index.mdx`: `### [editor]` with `command` (placeholders,
  resolution order, appended line forms) and `terminal`.
- `reference/how-it-works.mdx`: opening a file runs `git config --get
  core.editor` (only when no config/env editor is set), then the editor itself
  with the repo root as cwd; a terminal editor gets the whole terminal until it
  exits.
- `git/merge-rebase.mdx`: the "Open each conflicted file in your editor" step
  mentions <kbd>e</kbd>.
- `getting-started/navigation.mdx` / `git/palette.mdx` / `tour.mdx`: only if
  they list per-pane help-bar entries that now include "edit"; check with
  `grep -rn "stage hunks\|open link" docs-site/src/content`.
- `README.md` key table and `docs/configuration.md` (key table row plus a
  `## Editor` section with the TOML example).

Screenshots: the status, staging and details help bars change. Regenerate via
`CARGO_TARGET_DIR=target/agent cargo run -p tt-git-testkit --example docshots -- docs-site/screens/captures`
then `pnpm screens` in `docs-site/`; if pnpm/Chrome aren't available, list
which images are stale. No new dialog, so no new capture. `pnpm build` in
`docs-site/` must pass.

## Order of work

1. Core helpers + tests (patch.rs).
2. `editor.rs` + config + tests.
3. Command, keymap defaults, actions, pane handlers, `App` plumbing,
   `TestDriver` tests, snapshot updates.
4. Runner suspend/GUI spawn + e2e test (manual check in Windows Terminal with
   nvim and VS Code, in a scratch repo under `target/agent/dev`, never this
   checkout).
5. Docs and screenshots; `cargo insta test --workspace --check --unreferenced
   reject`, `cargo test --workspace --doc`, `cargo clippy --workspace
   --all-targets`, `cargo fmt --all`, all with `CARGO_TARGET_DIR=target/agent`.

## Out of scope

- <kbd>e</kbd> in the graph or branches panes (no file selected there).
- A socket/CLI method to open files, and plugin events for it.
- Opening the file as it was in an old commit (always the working-tree copy).

## Changes made while implementing

- Status pane line: in the default layout details covers the status pane,
  so it almost never holds the file's diff. Instead `e` loads the file's
  diffs in the background (`loaders::status_edit_target`, the same
  `stage_diff` calls as the staging view): the first unstaged change, else
  the first staged one carried through the unstaged hunks. A conflicted
  file opens at its first `<<<<<<<` line.
- The help-bar label is "edit file" (the palette shows "Edit file").
- `App::terminal_title()` now builds the `<repo> - tt-git` title, used by
  `main.rs` and by the runner after an editor exits.
- Work done in the detached worktree `.worktrees/open-in-editor` because
  another session was editing `app.rs`/`keymap.rs`/`action.rs` (undo/redo)
  in the primary checkout at the same time.
