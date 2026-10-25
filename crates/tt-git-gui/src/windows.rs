//! The app's windows: one per repository, each with its own
//! [`AppModel`](crate::model::AppModel), plus the start window
//! ([`crate::start`]) when no repository is open.
//!
//! Every way of opening a repository comes here: the command line, the
//! start window, "Open repository" (the native folder picker) and opening
//! another worktree. A repository that already has a window gets that
//! window brought forward instead of a second one. The app quits when its
//! last window closes ([`quit_when_last_closes`]).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AnyWindowHandle, App, Global, PathPromptOptions};
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_app::Action;
use tt_git_core::{GitCli, Repo};

use crate::window_state;

/// Makes the app for a repository: what `tt-git-gui`'s `main` does at
/// startup (config, themes, plugins), kept for every window opened later.
pub type NewApp = dyn Fn(Repo) -> Result<(tt_git_app::App, UnboundedReceiver<Action>), String>;

struct Launcher(Rc<NewApp>);

impl Global for Launcher {}

/// The repository windows opened, with their repositories' roots. Closed
/// windows are dropped from it whenever it's read.
#[derive(Default)]
struct RepoWindows(Vec<(AnyWindowHandle, PathBuf)>);

impl Global for RepoWindows {}

/// Remembers how to make the app for a repository, so the windows opened
/// later start as if tt-git had been started there.
pub fn set_launcher(new_app: Rc<NewApp>, cx: &mut App) {
    cx.set_global(Launcher(new_app));
}

/// Quits once the last window (repository or start) is closed, writing
/// what [`window_state`] remembers first.
pub fn quit_when_last_closes(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            window_state::flush(cx);
            cx.quit();
        }
    })
    .detach();
}

/// Notes that `window` shows the repository at `root` (called by
/// [`open_window`](crate::open_window)).
pub(crate) fn register(window: AnyWindowHandle, root: &Path, cx: &mut App) {
    window_state::remember_repo(root, cx);
    cx.default_global::<RepoWindows>().0.push((window, canonical(root)));
}

/// The open repository windows and their roots.
pub fn repo_windows(cx: &mut App) -> Vec<(AnyWindowHandle, PathBuf)> {
    let open = cx.windows();
    let windows = cx.default_global::<RepoWindows>();
    windows.0.retain(|(w, _)| open.contains(w));
    windows.0.clone()
}

/// The window showing the repository at `root`, if one does.
pub fn window_for(root: &Path, cx: &mut App) -> Option<AnyWindowHandle> {
    let root = canonical(root);
    repo_windows(cx).into_iter().find(|(_, r)| *r == root).map(|(w, _)| w)
}

/// Opens `repo` in a new window, or brings forward the window that shows
/// it already.
pub fn open_repo(repo: Repo, cx: &mut App) -> Result<AnyWindowHandle, String> {
    if let Some(window) = window_for(repo.root(), cx) {
        tracing::info!("{} is open already", repo.root().display());
        let _ = window.update(cx, |_, window, _| window.activate_window());
        return Ok(window);
    }
    let Some(launcher) = cx.try_global::<Launcher>().map(|l| l.0.clone()) else {
        return Err("no launcher".into());
    };
    // So plugins start where tt-git is, as the TUI does.
    if let Err(e) = std::env::set_current_dir(repo.root()) {
        tracing::warn!("couldn't change to {}: {e}", repo.root().display());
    }
    let root = repo.root().to_path_buf();
    tracing::info!("opening {}", root.display());
    let (app, rx) = launcher(repo)?;
    crate::open_window(app, rx, cx)
        .map(Into::into)
        .map_err(|e| format!("couldn't open a window for {}: {e}", root.display()))
}

/// Opens the repository at `path` (or the one it's inside), as
/// [`open_repo`] does. A folder that isn't in a repository is an error
/// that says so.
pub fn open_path(path: &Path, cx: &mut App) -> Result<AnyWindowHandle, String> {
    let repo = Repo::discover(path, &GitCli::new(path)).map_err(|e| format!("can't open {}:\n{e}", path.display()))?;
    open_repo(repo, cx)
}

/// Asks for folders with the platform's folder picker and opens each one.
/// `done` gets how many opened (0 when the picker was cancelled) or what
/// went wrong: the picker didn't open, or a folder isn't a repository.
pub fn pick_repos(cx: &mut App, done: impl FnOnce(Result<usize, String>, &mut App) + 'static) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: Some("Open".into()),
    });
    cx.spawn(async move |cx| {
        let result = match chosen.await {
            Ok(Ok(paths)) => Ok(paths.unwrap_or_default()),
            Ok(Err(e)) => Err(format!("couldn't show the folder picker: {e}")),
            Err(_) => Ok(Vec::new()),
        };
        cx.update(|cx| {
            let outcome = result.and_then(|paths| {
                let mut opened = 0;
                let mut errors = Vec::new();
                for path in paths {
                    match open_path(&path, cx) {
                        Ok(_) => opened += 1,
                        Err(e) => errors.push(e),
                    }
                }
                if errors.is_empty() {
                    Ok(opened)
                } else {
                    Err(errors.join("\n\n"))
                }
            });
            if let Err(e) = &outcome {
                tracing::warn!("{e}");
            }
            done(outcome, cx);
        });
    })
    .detach();
}

/// `path` with symlinks resolved, so two spellings of one repository match.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::component::Root;
    use gpui::{TestAppContext, WindowHandle};
    use tt_git_app::keymap::Command;
    use tt_git_app::{App as TtApp, Config};
    use tt_git_core::time::FixedClock;
    use tt_git_testkit::{NOW, Sandbox};

    use super::*;
    use crate::testing::Gui;

    fn sandbox(file: &str) -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file(file, "a", "first");
        sb
    }

    /// GPUI set up with a launcher that makes the app as the harness does.
    fn setup(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::init(cx);
            set_launcher(
                Rc::new(|repo| {
                    let config = Config {
                        watch: false,
                        ..Config::default()
                    };
                    TtApp::new(config, repo, Arc::new(FixedClock(NOW))).map_err(|e| e.to_string())
                }),
                cx,
            );
        });
    }

    fn open(cx: &mut TestAppContext, sb: &Sandbox) -> Gui {
        let window = cx.update(|cx| open_path(sb.path(), cx)).expect("opens");
        Gui::attach(cx, window.downcast::<Root>().expect("a repo window"))
    }

    fn roots(cx: &mut TestAppContext) -> Vec<PathBuf> {
        cx.update(|cx| repo_windows(cx).into_iter().map(|(_, r)| r).collect())
    }

    fn open_repository(gui: &mut Gui) {
        gui.model.update(&mut gui.cx, |m, cx| {
            m.send(Action::Command(Command::OpenRepository), cx)
        });
        gui.draw();
    }

    #[gpui::test]
    fn a_second_repo_opens_a_second_window_and_closing_one_leaves_the_other(cx: &mut TestAppContext) {
        setup(cx);
        let (a, b) = (sandbox("a.txt"), sandbox("b.txt"));
        let mut first = open(cx, &a);
        let mut second = open(cx, &b);
        assert_eq!(cx.windows().len(), 2);
        assert_eq!(roots(cx), [canonical(a.path()), canonical(b.path())]);
        assert_ne!(first.model, second.model, "each window has its own app");

        // Each window takes its own keys.
        second.press("q");
        assert!(!second.is_open());
        assert!(first.is_open());
        first.press("j");
        assert_eq!(roots(cx), [canonical(a.path())]);
        assert_eq!(
            canonical(&first.read(|app| app.repo().root().to_path_buf())),
            canonical(a.path())
        );
    }

    #[gpui::test]
    fn opening_a_repo_that_has_a_window_brings_that_one_forward(cx: &mut TestAppContext) {
        setup(cx);
        let a = sandbox("a.txt");
        let gui = open(cx, &a);
        let again = cx.update(|cx| open_path(&a.path().join("."), cx)).expect("opens");
        assert_eq!(again, AnyWindowHandle::from(gui.window));
        assert_eq!(cx.windows().len(), 1);
    }

    #[gpui::test]
    fn a_new_window_is_moved_along_from_the_open_ones(cx: &mut TestAppContext) {
        setup(cx);
        let (a, b) = (sandbox("a.txt"), sandbox("b.txt"));
        let first = open(cx, &a);
        let second = open(cx, &b);
        let origin = |gui: &Gui, cx: &mut TestAppContext| gui.window.update(cx, |_, w, _| w.bounds().origin).unwrap();
        let (one, two) = (origin(&first, cx), origin(&second, cx));
        assert_eq!(two.x - one.x, gpui::px(window_state::CASCADE));
        assert_eq!(two.y - one.y, gpui::px(window_state::CASCADE));
    }

    #[gpui::test]
    fn open_repository_asks_for_a_folder_and_opens_it_in_a_new_window(cx: &mut TestAppContext) {
        setup(cx);
        let (a, b) = (sandbox("a.txt"), sandbox("b.txt"));
        let mut gui = open(cx, &a);
        open_repository(&mut gui);
        assert!(cx.did_prompt_for_paths());
        let wanted = b.path().to_path_buf();
        cx.simulate_path_prompt_response(|options| {
            assert!(options.directories && !options.files);
            Some(vec![wanted])
        });
        cx.run_until_parked();
        assert_eq!(roots(cx), [canonical(a.path()), canonical(b.path())]);
        assert!(gui.is_open(), "the first window stays");
    }

    #[gpui::test]
    fn picking_a_folder_outside_a_repo_shows_why_in_the_window(cx: &mut TestAppContext) {
        setup(cx);
        let a = sandbox("a.txt");
        let mut gui = open(cx, &a);
        let elsewhere = tempfile::tempdir().unwrap();
        open_repository(&mut gui);
        let wanted = elsewhere.path().to_path_buf();
        cx.simulate_path_prompt_response(|_| Some(vec![wanted]));
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        let error = gui.read(|app| app.error().map(str::to_string)).unwrap_or_default();
        assert!(error.starts_with("can't open "), "{error}");
    }

    #[gpui::test]
    fn cancelling_the_picker_opens_nothing(cx: &mut TestAppContext) {
        setup(cx);
        let a = sandbox("a.txt");
        let mut gui = open(cx, &a);
        open_repository(&mut gui);
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        assert_eq!(gui.read(|app| app.error().map(str::to_string)), None);
    }

    #[gpui::test]
    fn the_start_window_opens_a_recent_repo_and_closes(cx: &mut TestAppContext) {
        setup(cx);
        let a = sandbox("a.txt");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(window_state::FILE);
        let gone = dir.path().join("gone");
        let mut state = window_state::GuiState::default();
        state.add_recent(&gone);
        state.add_recent(a.path());
        state.save(&file).unwrap();
        cx.update(|cx| window_state::install(file.clone(), cx));

        let start: WindowHandle<Root> = cx.update(|cx| crate::start::open(None, cx)).unwrap();
        let view = start
            .update(cx, |root, _, _| {
                root.view().clone().downcast::<crate::start::StartView>().unwrap()
            })
            .unwrap();
        let recent = view.read_with(cx, |v, _| v.recent().to_vec());
        assert_eq!(recent.len(), 2);
        assert!(!recent[0].missing && recent[1].missing, "{recent:?}");

        // The missing one fails, says why, and is forgotten.
        start
            .update(cx, |_, window, cx| {
                view.update(cx, |v, cx| v.open_recent(1, window, cx))
            })
            .unwrap();
        assert!(view.read_with(cx, |v, _| v.error().is_some()));
        assert_eq!(view.read_with(cx, |v, _| v.recent().len()), 1);
        assert_eq!(cx.update(|cx| window_state::state(cx)).recent, [a.path().to_path_buf()]);

        let mut visual = gpui::VisualTestContext::from_window(start.into(), cx);
        visual.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(roots(cx), [canonical(a.path())]);
        assert_eq!(cx.windows().len(), 1, "the start window closed");
    }

    #[gpui::test]
    fn the_window_size_is_remembered_for_the_next_window(cx: &mut TestAppContext) {
        setup(cx);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state").join(window_state::FILE);
        cx.update(|cx| window_state::install(file.clone(), cx));
        let a = sandbox("a.txt");
        let mut gui = open(cx, &a);
        gui.cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(700.)));
        gui.draw();
        let saved = cx.update(|cx| window_state::state(cx)).window.expect("remembered");
        assert_eq!((saved.width, saved.height), (900., 700.));
        assert_eq!(cx.update(|cx| window_state::state(cx)).recent, [a.path().to_path_buf()]);

        cx.update(window_state::flush);
        let on_disk = window_state::GuiState::load(&file);
        assert_eq!(on_disk.window, Some(saved));

        // The next run starts there.
        let next = cx.update(|cx| window_state::next_bounds(0, cx));
        assert_eq!(next.get_bounds().size, gpui::size(gpui::px(900.), gpui::px(700.)));
    }
}
