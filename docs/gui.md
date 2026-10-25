# The desktop app: porting an area

tt-git's desktop app is `crates/tt-git-gui`, a separate `tt-git-gui` binary
drawn with [GPUI](https://gpui.rs/) and gpui-component. It runs the same
app as the TUI: `tt-git-app` holds the state and logic, and each front end
only draws it and feeds it input. This page is for whoever ports an area
(a pane, the dialogs, the terminal panes) from the TUI to the GUI: what's
already there, which files are yours, and the steps.

Read [CLAUDE.md](../CLAUDE.md) first. Its build rules apply to everything
here: `CARGO_TARGET_DIR=target/agent` on every cargo command, your own
`TTGIT_CONFIG_DIR` and `TTGIT_STATE_DIR`, and a scratch repo.

## How the GUI works

| File | What it does |
|---|---|
| `src/main.rs` | Sets the app up as `tt-git`'s `main` does (config, themes, plugins) and opens the first window: the repository's, or the start window outside one. |
| `src/windows.rs` | Every way of opening a repository (command line, start window, "Open repository", another worktree) goes through `open_repo`/`open_path`: a new window with its own `AppModel`, or the window that shows it already brought forward. `pick_repos` is the native folder picker. The app quits when its last window closes. |
| `src/window_state.rs` | `<state_dir>/gui.json`: the last window's bounds and mode, and the recent repositories. `place` puts a new window on a display that still exists. Only `main` installs it, so tests never touch the state folder. |
| `src/start.rs` | The start window: recent repositories and "Open folder…". |
| `src/tt_git_bin.rs` | The `tt-git` plugins get as `TTGIT_BIN_PATH`. |
| `src/model.rs` | `AppModel`, the GPUI entity that owns `tt_git_app::App`. A task applies the app's actions, a timer sends `Action::Tick` every 250 ms, and after every input it carries out what the app asks: `take_redraw` (notify), `take_clipboard` (system clipboard), `take_editor` (a windowed editor starts detached, a terminal one in a pane: see [Editors](#editors)), `should_quit` (close the window). |
| `src/shell.rs` | `Shell`, the window's view. Draws the layout, the help bar and what opens over the panes, sends every key to tt-git's keymap, and reports where each pane went (`begin_frame`, `set_layout`), as the TUI's `draw_app` does. |
| `src/keys.rs` | GPUI's `Keystroke` as tt-git's `KeyCombo`. tt-git's keymap resolves every key, text input included. There is no GPUI action per command. |
| `src/theme.rs` | `GuiTheme`: tt-git's colors as `Hsla`, every `StyleRole` as a `HighlightStyle`, and gpui-component's theme set to match. |
| `src/font.rs` | `MonoFont`: the monospace font and its cell size. |
| `src/layout.rs` | The `[layout]` tree as resizable splits. Dragging a handle resizes them; each split keeps its sizes for the session, by its place and what it shows (`split_sizes` reads them). |
| `src/context_menu.rs` | The right-click menu every `PaneFrame` has: the focused pane's commands with their keys (`App::pane_commands`), each run as its key would. |
| `src/ui.rs` | `Ui`, what every view draws with, and `Measures`. |
| `src/frame.rs`, `src/rows.rs`, `src/text.rs`, `src/overlay.rs` | The building blocks (below). |
| `src/help_bar.rs` | The help bar. |
| `src/menus.rs`, `src/menus/` | The menus (below), the title bar (`bar.rs`) and the app icon (`icon.rs`, `assets/`, `build.rs`). |
| `src/panes/`, `src/dialogs/`, `src/terminal.rs` | One file per area (below). |
| `src/testing.rs` | The GPUI test harness. |
| `src/gpui_tokio.rs` | Zed's tokio adapter, copied in. Jobs run on its runtime. |

Things that work differently from the TUI, on purpose:

- **Sizes are in pixels, text in columns.** `geom::Rect` holds logical pixels
  in the GUI. View models still fit text to a width in columns, so the GUI
  draws everything in one monospace font and turns a measured size into
  columns and rows with `MonoFont::fit`. Places in `tt-git-app` that read a
  rect's size as terminal cells are marked `// GUI:` (see
  [Cell units](#cell-units)).
- **The app owns scrolling.** As in the TUI, a pane's `Cursor` keeps the
  scroll offset and the view model returns only the rows on screen. The
  GUI doesn't scroll lists itself: the wheel becomes `MouseKind::ScrollUp`
  and `ScrollDown` (one per 3 lines of movement) and the app scrolls.
- **One window per repository.** "Open repository" (in the palette and the
  Repository menu, from
  `Command::OpenRepository`, which only a front end that calls
  `App::enable_open_repository` offers) opens another in a window of its
  own. Opening another worktree still replaces the window, as the TUI
  does. Closing a window whose terminal editor is running asks first.
- **Sizes come from the last frame.** GPUI lays out after it renders, so a
  view asks how many rows fit where its list was laid out last frame
  (`Ui::fit`). When that changes, the window draws again at once. Before
  the first frame the answer is 0.

## Your files

Each area owns the files in its row. Edit only those. If you need a change
anywhere else, ask the lead (see [Shared files](#shared-files)).

| Area | `tt-git-app/src` | `tt-git-tui/src/draw` | `tt-git-gui/src` |
|---|---|---|---|
| B1 Graph | `view/graph.rs`, `panes/graph.rs` | `graph.rs` | `panes/graph.rs` |
| B2 Status | `view/status.rs`, `panes/status.rs` | `status.rs` | `panes/status.rs` |
| B3 Branches | `view/branches.rs`, `panes/branches.rs` | `branches.rs` | `panes/branches.rs` |
| B4 Details | `view/details.rs`, `panes/details.rs` | `details.rs` | `panes/details.rs`, `panes/details/diff.rs` |
| B5 Stage | `view/stage.rs`, `panes/stage.rs` | `stage.rs` | `panes/details/stage.rs` |
| B6 Blame | `view/blame.rs`, `panes/blame.rs` | `blame.rs` | `panes/details/blame.rs` |
| B7 Dialogs | `view/dialogs.rs`, `modal.rs`, `filter.rs` | `modal.rs` (palette, prompts, confirms, discard-all, help popup, error popup), `filter.rs` | `dialogs/*.rs` except `dialogs/mod.rs` |
| B8 Terminal panes | `view/terminal.rs`, `pty_pane.rs` | `pty.rs` | `terminal.rs` |

Every `view/<area>.rs` is declared in `view.rs` already, and every GUI file
is registered already (`panes/mod.rs`, `dialogs/mod.rs`, the shell), so a
port never edits those. Need more files? Make them submodules of your own:
`view/details/files.rs` declared in `view/details.rs`, say.

B4, B5 and B6 all draw in the details pane. `panes/mod.rs` picks the file:
staging goes to `details/stage.rs`, blame to `details/blame.rs`, anything
else to `details.rs`. Each draws the whole pane, its `PaneFrame` included.

### Shared files

Don't edit these. Ask the lead, who changes them on `gui` for everyone.

- `tt-git-app`: `view.rs` (`StyleRole`, `TextSpan`), `view/code.rs`,
  `theme.rs`, `style.rs`, `widgets.rs`, `app.rs` and `app/`, `keymap.rs`,
  `mouse.rs`, `geom.rs`, `layout.rs`, `component.rs`.
- `tt-git-tui`: `draw/mod.rs`, `draw/spans.rs`, `draw/layout.rs`,
  `tui_theme.rs`, `input.rs`, `runner.rs`, `convert.rs`.
- `tt-git-gui`: everything not in the table above, `Cargo.toml` included.
- `Cargo.toml`, `Cargo.lock`, `.github/`, `docs/`, `docs-site/`.

If you're sure a change to a shared file is small and only yours (a new
`pub` on a pane field your view model reads, say), make it, keep it to one
line, and say so in your report.

## The building blocks

All of them have rustdoc; `cargo doc -p tt-git-gui --open` with the agent
target dir.

- **`PaneCx`** (`panes/mod.rs`): what a pane's `render` gets. `app` is
  `&mut tt_git_app::App` (for `set_viewport`, `note_drawn`), `ui` is the
  `Ui`, `pane` and `focused` say which pane and whether it has the focus.
  Dialogs get `DialogCx` (`app`, `ui`).
- **`Ui`** (`ui.rs`): `theme` (`GuiTheme`), `font` (`MonoFont`),
  `fit(id)` (the columns and rows where the element measured as `id` was
  laid out), `measure(id)` (an element that records its parent's bounds as
  `id`; put it in a `.relative()` parent), and `send_mouse(MouseInput, cx)`.
  Clone it into event handlers.
- **`PaneFrame`** (`frame.rs`): the bordered pane with `[n] title` on top,
  in the focus colors when focused. `.title(text)` or `.title_spans(spans)`
  (each span laid over the title look, like the branches tabs), then
  `.child(...)` for the body. It measures itself as `frame_id(pane)` (that's
  what the shell reports) and its body as `body_id(pane)`. A click or wheel
  that nothing inside handles goes to the app as on the border, which
  focuses the pane. More slots:
  - `.title_right(spans)`: spans pushed to the right end of the title line
    (blame's date and recency). The title is cut short before them.
    `title_cols(ui, pane)` is the columns the title line had after `[n] `
    last frame (it's measured as `title_id(pane)`), to fit both: the
    details pane picks its right part with `view::details::fit_title_right`.
  - `.on_title_click(|col| ...)`: what a click on column `col` of the title
    (after `[n] `) is for the app, or `None`. The column comes from where
    the text was laid out (GPUI's `TextLayout`), so it's exact whatever the
    font. The branches tabs use it.
  - `.bottom(left, right)`: a line along the bottom, under the body, for
    what the TUI writes on a pane's bottom border (the branches pane's
    conflicts and worktree sizes). Not drawn while both are empty.
  - A right-click menu (`context_menu.rs`), on every pane.
- **`StyledRows`** (`rows.rs`): a list of one-line rows filling its parent.
  `StyledRows::new(ui, id, pane, rows)` measures itself as `id`, sends a
  click on line `n` as `MouseKind::Click` at body row `n` (a right click as
  `MouseKind::RightClick`, before the pane's menu opens), and the wheel as
  scroll steps. `.first_body_row(k)` when the pane draws `k` lines above the
  list; `.on_target(|line| ...)` for a list that is a part of its own
  (`PanePart::FileRow(line)` for the details file list).
- **`StyledRow`** (`rows.rs`): one row. `StyledRow::new(theme, spans, row)`
  where `row` is the look of the whole row, laid under the spans as the TUI
  patches a line's style: `text::selection(picked, focused)` for a picked
  row, or a role such as `StyleRole::DiffAddLine`.
- **`StyledLine`** (`text.rs`): a line of spans as GPUI text, for anything
  that isn't a list row (titles, headers, a prompt's input).
- **`overlay::centered(content)`**: a layer over the main area with
  `content` in its middle, for dialogs.
- **`GuiTheme`** (`theme.rs`): `role(role)` and `span(span)` give a
  `HighlightStyle`; `fg(role)` and `bg(role)` give colors; `background` and
  `foreground` are the window's.
- **`MonoFont`** (`font.rs`): `family`, `size`, `line_height`,
  `cell_width`, and `fit(size)`.

### Style roles

A view model never carries a style, only a `StyleRole` per span, and the
front end looks it up in the theme. There's a role for every `Theme` field:

`Plain`, `Muted`, `Error`, `Underlined`, `Border`, `BorderFocused`,
`Title`, `TitleFocused`, `Selected`, `SelectedUnfocused`, `HelpKey`,
`HelpKeyAccel` (the underlined letter of `[y]es`), `HelpText`, `Busy`,
`Lane(n)`, `LabelHead`, `LabelDetached`, `LabelLocal`, `LabelRemote`,
`LabelTag`, `Author`, `Date`, `Sha`, `DiffAdd`, `DiffDel`, `DiffHunk`,
`DiffHeader`, `DiffAddLine`, `DiffDelLine`, `Code(Class)`, `BlameHeat(n)`
(0 newest), `BlameAuthor(n)`, `BlameUncommitted`, `BlameLatest`,
`BlameOlder`, `SearchMatch`.

A span can lay a second role over its own: `TextSpan::with_over(role)`.
Code on a changed line and search matches use it. `view::code::code_spans`
turns code (syntax tokens and search matches) into spans the way the TUI's
`marked_code_spans` draws it; a test in `draw/spans.rs` checks the two look
the same.

If your area needs a look no role gives, ask the lead for a role rather
than putting a style in your view model.

## Porting an area, step by step

`view/graph.rs` and `draw/graph.rs` are the template: read them first.

1. **Move the view model into `tt-git-app/src/view/<area>.rs`.** Take what
   the TUI's `draw/<area>.rs` decides (which text, in which role, which row
   is picked, what's cut to fit) and make it a function on the pane state
   that returns plain data: rows of `TextSpan`s plus whatever the front end
   needs to lay them out (the visible range, a header, the selection). Keep
   ratatui and crossterm out: `tt-git-app/tests/front_end_free.rs` fails if
   they get in. The pane's `set_viewport` stays the one place that scrolls.
2. **Redraw the TUI from it.** `draw/<area>.rs` calls the view model and only
   maps roles to styles (`theme.span(&span)`) and lays the result out. The
   insta snapshots must not change. If one does, the view model is wrong,
   not the snapshot.
3. **Write the view model's unit tests** next to it, as `view/graph.rs`
   does: the roles, the fitting to a width, the selection, the empty and
   loading states. Move the TUI's tests of the same logic there.
4. **Build the GUI view** in your file in `tt-git-gui`. The pattern:

   ```rust
   pub const ROWS: &str = "status";

   pub fn render(cx: &mut PaneCx) -> AnyElement {
       let (ui, focused) = (cx.ui, cx.focused);
       let pane = cx.app.status_mut();
       let fit = ui.fit(ROWS);
       pane.set_viewport(fit.rows);
       // `view` and `title` are what your view model provides.
       let view = pane.view(fit.rows, fit.cols);
       let rows = view
           .rows
           .iter()
           .map(|r| StyledRow::new(&ui.theme, &r.spans, selection(r.picked, focused)))
           .collect();
       PaneFrame::new(ui, PaneId::Status, focused)
           .title(pane.title())
           .child(StyledRows::new(ui, ROWS, PaneId::Status, rows))
           .into_any_element()
   }
   ```

   Clicks and the wheel go to the app as `MouseInput`, with the same
   `MouseTarget` the TUI's `input.rs` would make, so the app handles them
   the same way. Anything with a key in the TUI keeps working with no GUI
   code: the keymap handles it.
5. **Write GUI tests** in a `#[cfg(test)] mod tests` at the bottom of your
   GUI file, with the harness in `testing.rs`:

   ```rust
   #[gpui::test]
   fn shows_the_changed_files(cx: &mut TestAppContext) {
       let sb = Sandbox::new();
       sb.commit_file("a.txt", "a", "first");
       sb.write("a.txt", "changed");
       let mut gui = Gui::open(cx, &sb);
       gui.press("tab");
       assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
       let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
       assert!(fit.rows > 0, "the list was drawn");
   }
   ```

   `Gui::open(cx, &sb)` opens the real window on a `Sandbox` repo and waits
   until the first loads are drawn. `press("ctrl+l")` sends a key in the
   keymap's syntax; `keystrokes("ctrl-l")` sends GPUI keystrokes through
   the window's real dispatch. Both wait until the app is idle (no jobs in
   flight, everything applied and drawn). `read(|app| ...)` reads the
   state; `gui.model.update(&mut gui.cx, ...)` gets `&mut App` to call a
   view model. GPUI tests check state, not pixels, and run headless on
   every OS.
6. **Look at it.** Run the binary on a scratch repo (below) and check the
   area against the TUI on the same repo.

## Commands

From the worktree root:

```sh
# Your area's GUI tests, and all of them
CARGO_TARGET_DIR=target/agent cargo test -p tt-git-gui panes::status
CARGO_TARGET_DIR=target/agent cargo test -p tt-git-gui

# The view model's tests
CARGO_TARGET_DIR=target/agent cargo test -p tt-git-app view::status

# Everything (TUI snapshots, all tests, doc tests), lint and format
CARGO_TARGET_DIR=target/agent cargo insta test --workspace --check --unreferenced reject
CARGO_TARGET_DIR=target/agent cargo test --workspace --doc
CARGO_TARGET_DIR=target/agent cargo clippy --workspace --all-targets
cargo fmt --all
```

The first build of `tt-git-gui` takes several minutes: GPUI is large.

To run it, make a scratch repo in your scratchpad, then:

```sh
env -u TTGIT_SOCKET_PATH -u TTGIT_REPO_PATH \
  TTGIT_CONFIG_DIR=target/agent/dev/config TTGIT_STATE_DIR=target/agent/dev/state \
  CARGO_TARGET_DIR=target/agent cargo run -p tt-git-gui -- --no-plugins <scratch repo>
```

It logs to `target/agent/dev/state/logs/tt-git-gui.log` (`TTGIT_LOG=debug`
logs every key).

CI builds and tests `tt-git-gui` on Windows, macOS and Linux in its own
workflow, `.github/workflows/gui.yml`. The TUI's jobs in `ci.yml` leave it
out. On Linux GPUI needs system packages; `gui.yml` lists them.

## The pass/fail test

An area is done when:

- `cargo insta test --workspace --check --unreferenced reject` passes with
  **no TUI snapshot changes**.
- The view model has unit tests in `tt-git-app`, and the GUI view has
  `#[gpui::test]`s in its file, and they pass.
- `cargo test --workspace --doc`, `cargo clippy --workspace --all-targets`
  and `cargo fmt --all` are clean.
- `tt-git-gui` builds, and you've run it and looked at your area.
- You touched only your files, or said which shared line you changed and
  why.

Rebase on `gui` before you finish. Commit as CLAUDE.md says:
`refactor(<area>): ...` for moving the view model and redrawing the TUI
from it, `feat(gui): ...` for the GUI view.

## Cell units

`geom::Rect` is in the front end's units: terminal cells in the TUI, logical
pixels in the GUI. The one place that needs cells is a PTY's size, and it
gets it from `view::terminal::pty_size(rect, cell_size)`, with the front
end's cell size from `App::set_cell_size` (1×1, the default, in the TUI).
`App::open_pane` and `App::open_agent_pane` both size their PTYs that way;
the GUI's terminal panes also resize the PTY to the rows and columns that
fit where they were laid out.

Still in cells: a plugin manifest's pane `width` and `height`
(`tt-git-plugin`'s `Dimension`) and `PtyPane::resize`, which takes rows
and columns.

Focus moves (`ctrl+h/j/k/l`) compare rects only with each other, and mouse
input arrives already hit-tested, so they work in either unit.

## Editors

`App::take_editor` hands the front end an `EditorLaunch`. The TUI suspends
itself and runs a terminal editor (`launch.terminal`) in its own terminal.
The GUI has no terminal to give, so `AppModel::after_input` calls
`App::open_editor_pane` (`app/editor_pane.rs`): the editor runs in a PTY
pane with the overlay placement, over the main area, titled with the
command and saying "quit <editor> to come back". Every key goes to it. The
release key (f12) doesn't leave it: the first press says the editor is
still running, a second in a row stops it ("stopped <editor>"). When the
editor exits the pane closes and the app applies
`EditorFinished(Ok(exit code))` itself, so what follows (the refresh, the
"editor exited with status N" notice) is the same as in the TUI. If it
can't start, the GUI sends `EditorFinished(Err)` and the error popup says
so. An editor with a window of its own starts detached and reports
`EditorFinished(Ok(None))` at once, as in the TUI.

The tests in `tt-git-gui/src/terminal/editor_tests.rs` use the test
binary itself as the terminal editor (the ignored test `fake_editor`), so
they run on every OS.

## Menus and the title bar

The menus list tt-git's commands; the keymap stays the only place keys
are bound. `menus::build(app)` makes them from the app as it is: tt-git
(About, Theme, Plugin logs, Quit; Update tt-git only where the app
`self_updates()`, which the GUI turns off), Repository, View, and a
menu named after the focused pane's key context with its `commands()`
(the help popup's first section). Each item shows the key
`display_key_text` gives in the focused context, so a rebind shows.
While a dialog, the help popup or an error is open, the command items are
disabled, since a key would go there too.

Every command item carries one action, `menus::RunCommand(Command)`. The
shell handles it by sending `Action::Command`, which is what a bound key
does. No GPUI key binding is made: the key is part of the item's text.
`About` and `Quit` are the only other actions; `Quit` sends the quit
command to every window.

Per platform (gpui-pre 0.3.9):

- **macOS**: `cx.set_menus` makes the native menu bar, installed by the
  active window when its menus change. The key follows the label,
  `Checkout  (o)`.
- **Windows and Linux**: GPUI keeps the menus but shows no menu bar, so
  the shell draws them in its title bar (`menus::bar`, gpui-component's
  `TitleBar` with a dropdown per menu). A picked item gives the focus back
  to the shell and dispatches the item's action to it. While a menu is
  open the shell ignores keys.

The window's title bar is always drawn by the app (`TitleBar` moves the
window and, under client-side decorations such as GNOME on Wayland, draws
its controls), with the title in it. The icon: on X11 through
`WindowOptions::icon`, on Windows as the executable's icon resource
(`build.rs`); Wayland and macOS take it from the packages
(`packaging/`): on Wayland from `share/applications/tt-git.desktop` and
the hicolor icons in the Linux tarball and AppImage (the file name is the
`app_id`, `tt-git`), on macOS from `tt-git.app/Contents/Resources/tt-git.icns`.

## Gotchas

- A test helper that opens a `Gui` must hand the `Sandbox` back with it. If
  the `Sandbox` is dropped, its repo is deleted and later git jobs fail
  with "could not run git: No such file or directory".
- `Gui::press` and `keystrokes` wait until the app is idle after each key.
  To check what happens when keys arrive back to back, call
  `model.key(..)` in a loop instead (see the shell's tests).

## Known gaps

- On Wayland the compositor places windows, so only the size (and
  maximized or fullscreen) comes back, not the position.
- The folder picker needs the desktop portal on Linux
  (`xdg-desktop-portal`); without it "Open repository" says it couldn't
  show the picker, and the start window can still open a recent
  repository. There's no way to type a path instead.
- Every window's app runs in one process, which has one current
  directory: opening a repository changes it to that one (so plugins
  start there, as in the TUI). The API socket of a closed window keeps
  listening until the app quits.
- `TTGIT_BIN_PATH` is the `tt-git` next to `tt-git-gui` or on `PATH`;
  with neither, plugins get none.
- Quit (the menu's, or cmd+Q on macOS) doesn't ask about running
  terminal editors; closing their window does.
- Wayland and macOS show no app icon when run from a cargo build: the
  icon comes with the packages (`tt-git.desktop` and the hicolor icons,
  `tt-git.app`'s `tt-git.icns`).
- Split sizes last for the session only; nothing saves them yet.

## The mouse

The app does what the mouse means, for both front ends; a front end only
says what the pointer was over (`MouseTarget`, with a `PanePart` for parts
of a pane with clicks of their own: a branches tab, a details file row, the
diff, a staging row).

- **Double click**: the app spots two clicks on the same row and runs the
  pane's `Component::double_click` command, what `enter` does on it.
- **Right click**: the GUI sends `MouseKind::RightClick` (the TUI leaves the
  right button to the terminal), which focuses the pane and picks the row as
  a click does, never half of a double click. Then gpui-component's context
  menu opens with `App::pane_commands`. A list that takes the right click
  calls `window.prevent_default()` rather than stopping the event, so the
  frame doesn't send it again and the menu still opens. While the menu has
  the focus, the shell sends the app no keys.
