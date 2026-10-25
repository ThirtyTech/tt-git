//! Updating the desktop app: "Check for Updates…" in the app menu (and
//! "Update tt-git" in the palette), a check in the background once a day,
//! and the restart that brings every window back on the new version.
//!
//! - [`install`] works out which download updates this copy and swaps it in.
//! - [`instances`] finds the other running copies and restarts them all.
//! - [`Updater`] is what goes online and touches files. `main` installs the
//!   real one ([`Releases`]); tests install a fake, so they never download
//!   anything or replace a binary. With none installed, checking says so.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, AsyncApp, Global, PromptLevel, WeakEntity};
use serde::{Deserialize, Serialize};
use tt_git_app::Action;
use tt_git_core::update::{CURRENT_VERSION, is_newer};

use crate::model::AppModel;

pub mod install;
pub mod instances;

use install::{Install, Refusal, Staged};
use instances::Instance;

/// What the update goes through to reach the network and the disk.
pub trait Updater: Send + Sync + 'static {
    /// The latest release's tag, such as `v0.9.0`.
    fn latest(&self) -> Result<String, String>;
    /// How this copy was installed, or why it can't update itself.
    fn install(&self) -> Result<Install, Refusal>;
    /// Downloads release `tag` for `install` and swaps it in (or stages
    /// its installer).
    fn prepare(&self, tag: &str, install: &Install) -> Result<Staged, String>;
    /// Starts the waiter that brings every instance back once they've quit
    /// ([`instances::finish`]). `registry` is the instances folder.
    fn start_waiter(&self, install: &Install, staged: &Staged, registry: &Path) -> Result<(), String>;
}

/// The button that installs the update.
pub const UPDATE: &str = "Update and Restart";
/// The button that opens the release page instead.
pub const OPEN_PAGE: &str = "Open Download Page";

/// How often the background check goes online.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after starting the background check first looks.
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// How often the background check looks whether a day has passed.
const CHECK_WAKE: Duration = Duration::from_secs(60 * 60);
/// How often an instance looks for a restart request.
const RESTART_POLL: Duration = Duration::from_secs(1);

/// The file in the state folder the background check keeps its time in,
/// shared by every instance.
pub const CHECK_FILE: &str = "gui-update.json";

/// What [`CHECK_FILE`] holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckState {
    /// When the last check went online, in seconds since the epoch.
    #[serde(default)]
    pub checked_at: u64,
    /// The latest release it found.
    #[serde(default)]
    pub latest: Option<String>,
}

impl CheckState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Whether a day has passed since the last check, at `now` (seconds).
    /// A clock that went back counts as due.
    pub fn due(&self, now: u64) -> bool {
        now < self.checked_at || now - self.checked_at >= CHECK_EVERY.as_secs()
    }
}

/// The updater, this process's place among the instances, and what's
/// going on.
#[derive(Default)]
struct UpdateState {
    updater: Option<Arc<dyn Updater>>,
    instance: Option<Instance>,
    /// A check or update is running.
    busy: bool,
    /// The release the help bar last announced.
    announced: Option<String>,
}

impl Global for UpdateState {}

/// Installs `updater`, for checks from the menu and the palette.
pub fn set_updater(updater: Arc<dyn Updater>, cx: &mut App) {
    cx.default_global::<UpdateState>().updater = Some(updater);
}

/// Joins the running instances as `instance`, so an update restarts this
/// one too: from now on it looks for a restart request every second.
pub fn join(instance: Instance, cx: &mut App) {
    cx.default_global::<UpdateState>().instance = Some(instance);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(RESTART_POLL).await;
            let asked = cx.update(|cx| {
                cx.try_global::<UpdateState>()
                    .and_then(|s| s.instance.as_ref())
                    .is_some_and(Instance::restart_requested)
            });
            if asked {
                tracing::info!("another tt-git updated: restarting");
                cx.update(restart_here);
            }
        }
    })
    .detach();
}

/// Leaves the running instances (the app is quitting).
pub fn leave(cx: &mut App) {
    if let Some(instance) = cx.try_global::<UpdateState>().and_then(|s| s.instance.as_ref()) {
        instance.leave();
    }
}

/// Checks in the background once a day (the time is kept in `file`, so
/// every instance shares it), and says in each window's help bar when a
/// newer release is out, once per release.
pub fn check_in_background(file: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FIRST_CHECK).await;
        loop {
            let Some(updater) = cx.update(updater) else {
                return;
            };
            let path = file.clone();
            let latest = cx
                .background_executor()
                .spawn(async move { background_check(&*updater, &path, instances::now_ms() / 1000) })
                .await;
            if let Some(tag) = latest {
                cx.update(|cx| announce(&tag, cx));
            }
            cx.background_executor().timer(CHECK_WAKE).await;
        }
    })
    .detach();
}

/// Goes online if a check is due, and returns the latest release known.
pub fn background_check(updater: &dyn Updater, file: &Path, now: u64) -> Option<String> {
    let mut state = CheckState::load(file);
    if state.due(now) {
        match updater.latest() {
            Ok(tag) => state.latest = Some(tag),
            Err(e) => tracing::info!("background update check: {e}"),
        }
        // A failed check waits a day too: offline is offline.
        state.checked_at = now;
        if let Err(e) = state.save(file) {
            tracing::warn!("couldn't write {}: {e}", file.display());
        }
    }
    state.latest
}

/// Says in every window's help bar that `tag` is out, if it's newer and
/// hasn't been said.
fn announce(tag: &str, cx: &mut App) {
    let state = cx.default_global::<UpdateState>();
    if !is_newer(tag, CURRENT_VERSION) || state.announced.as_deref() == Some(tag) {
        return;
    }
    state.announced = Some(tag.to_string());
    let text = format!("tt-git {tag} is out: tt-git ▸ Check for Updates installs it");
    for model in crate::windows::models(cx) {
        model.update(cx, |m, cx| m.send(Action::Notify(text.clone()), cx));
    }
}

fn updater(cx: &mut App) -> Option<Arc<dyn Updater>> {
    cx.try_global::<UpdateState>().and_then(|s| s.updater.clone())
}

fn send(model: &WeakEntity<AppModel>, action: Action, cx: &mut App) {
    let _ = model.update(cx, |m, cx| m.send(action, cx));
}

/// "Check for Updates…": looks for a newer release and offers to install
/// it, in prompts over `window`. What happens shows in `model`'s help bar,
/// and a failure in its error popup.
pub fn check(window: AnyWindowHandle, model: WeakEntity<AppModel>, cx: &mut App) {
    let Some(updater) = updater(cx) else {
        send(
            &model,
            Action::Error("this build of tt-git can't update itself".into()),
            cx,
        );
        return;
    };
    let state = cx.default_global::<UpdateState>();
    if state.busy {
        send(
            &model,
            Action::Notify("already checking for a tt-git update".into()),
            cx,
        );
        return;
    }
    state.busy = true;
    send(&model, Action::Notify("checking for a tt-git update…".into()), cx);
    cx.spawn(async move |cx| {
        let outcome = offer(updater, window, &model, cx).await;
        cx.update(|cx| {
            cx.default_global::<UpdateState>().busy = false;
            if let Err(e) = outcome {
                tracing::warn!("{e}");
                send(&model, Action::Error(e), cx);
            }
        });
    })
    .detach();
}

/// Asks over `window`; `None` when the window has gone.
async fn ask(
    window: AnyWindowHandle,
    level: PromptLevel,
    message: String,
    detail: String,
    answers: &[&str],
    cx: &mut AsyncApp,
) -> Option<usize> {
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(level, &message, Some(&detail), answers, cx)
        })
        .ok()?;
    answer.await.ok()
}

async fn offer(
    updater: Arc<dyn Updater>,
    window: AnyWindowHandle,
    model: &WeakEntity<AppModel>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let u = updater.clone();
    let latest = cx
        .background_executor()
        .spawn(async move { u.latest() })
        .await
        .map_err(|e| format!("checking for a tt-git update failed:\n{e}"))?;
    let current = CURRENT_VERSION;
    if !is_newer(&latest, current) {
        cx.update(|cx| send(model, Action::Notify(format!("tt-git {current} is up to date")), cx));
        let detail = format!("You have {current}, and the latest release is {latest}.");
        ask(
            window,
            PromptLevel::Info,
            "tt-git is up to date".into(),
            detail,
            &["OK"],
            cx,
        )
        .await;
        return Ok(());
    }
    cx.update(|cx| cx.default_global::<UpdateState>().announced = Some(latest.clone()));
    cx.update(|cx| send(model, Action::Notify(format!("tt-git {latest} is available")), cx));
    let title = format!("tt-git {latest} is available");
    let install = match updater.install() {
        Ok(install) => install,
        Err(refusal) => {
            let detail = format!("You have {current}. {refusal} Download the new release from its page.");
            let answer = ask(window, PromptLevel::Info, title, detail, &[OPEN_PAGE, "Close"], cx).await;
            if answer == Some(0) {
                cx.update(|cx| cx.open_url(&install::release_page(&latest)));
            }
            return Ok(());
        }
    };
    let detail = format!(
        "You have {current}. Updating downloads it, replaces this copy and restarts tt-git: \
         every window closes and opens again on its repository."
    );
    if ask(window, PromptLevel::Info, title, detail, &[UPDATE, "Later"], cx).await != Some(0) {
        return Ok(());
    }

    cx.update(|cx| send(model, Action::Notify(format!("downloading tt-git {latest}…")), cx));
    let (u, tag, inst) = (updater.clone(), latest.clone(), install.clone());
    let staged = cx
        .background_executor()
        .spawn(async move { u.prepare(&tag, &inst) })
        .await
        .map_err(|e| format!("updating tt-git to {latest} failed:\n{e}"))?;
    tracing::info!("updated to {latest}: restarting");
    cx.update(|cx| send(model, Action::Notify(format!("restarting tt-git {latest}…")), cx));
    cx.update(|cx| restart_all(&*updater, &install, &staged, cx))
        .map_err(|e| {
            format!("tt-git {latest} is installed, but restarting failed:\n{e}\n\nQuit and start tt-git again.")
        })
}

/// Asks every instance to restart, starts the waiter that brings them back,
/// and restarts this one.
fn restart_all(updater: &dyn Updater, install: &Install, staged: &Staged, cx: &mut App) -> Result<(), String> {
    if let Some(instance) = cx.try_global::<UpdateState>().and_then(|s| s.instance.as_ref()) {
        instance
            .request_restart()
            .map_err(|e| format!("couldn't ask the other windows to restart: {e}"))?;
        let registry = instance.dir().to_path_buf();
        updater.start_waiter(install, staged, &registry)?;
    } else {
        updater.start_waiter(install, staged, Path::new(""))?;
    }
    restart_here(cx);
    Ok(())
}

/// Notes which repositories this instance has open and quits every window,
/// for the waiter to start it again.
fn restart_here(cx: &mut App) {
    // As each app opened it, not canonicalized (`\\?\C:\…` on Windows).
    let repos: Vec<PathBuf> = crate::windows::models(cx)
        .into_iter()
        .map(|m| m.read(cx).app().repo().root().to_path_buf())
        .collect();
    if let Some(instance) = cx.default_global::<UpdateState>().instance.as_mut()
        && let Err(e) = instance.restarting(repos)
    {
        tracing::error!("couldn't note the open repositories: {e}");
    }
    crate::menus::quit_all(cx);
}

/// The real [`Updater`]: GitHub's releases, the running binary.
pub struct Releases {
    /// Where downloads go (`<state_dir>/gui-update`).
    pub work: PathBuf,
}

impl Releases {
    fn current_exe() -> Result<PathBuf, Refusal> {
        let exe = std::env::current_exe().map_err(|e| Refusal::NoExe(e.to_string()))?;
        // The bundle's real path, not a link to it.
        Ok(if cfg!(target_os = "macos") {
            exe.canonicalize().unwrap_or(exe)
        } else {
            exe
        })
    }

    /// How the running copy was installed.
    pub fn detect() -> Result<Install, Refusal> {
        let exe = Self::current_exe()?;
        let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
        let install = install::detect(&exe, install::Os::CURRENT, appimage.as_deref())?;
        install::platform().ok_or(Refusal::NoBuild)?;
        Ok(install)
    }
}

impl Updater for Releases {
    fn latest(&self) -> Result<String, String> {
        tt_git_core::update::latest_release().map_err(|e| e.to_string())
    }

    fn install(&self) -> Result<Install, Refusal> {
        Self::detect()
    }

    fn prepare(&self, tag: &str, install: &Install) -> Result<Staged, String> {
        let platform = install::platform().ok_or_else(|| Refusal::NoBuild.to_string())?;
        // What an earlier update left (an installer, the waiter's copy).
        let _ = std::fs::remove_dir_all(&self.work);
        let work = self.work.join(tag);
        install::prepare(
            install,
            tag,
            platform,
            &install::Curl,
            &work,
            install::Os::CURRENT,
            &mut |step| tracing::info!("update: {step}"),
        )
    }

    fn start_waiter(&self, install: &Install, staged: &Staged, registry: &Path) -> Result<(), String> {
        let os = install::Os::CURRENT;
        let (waiter, installer) = match staged {
            // The new copy, which is in place.
            Staged::Replaced => {
                let waiter = match install {
                    Install::AppBundle { app } => app.join("Contents/MacOS/tt-git-gui"),
                    other => other.launch(os),
                };
                (waiter, None)
            }
            // The installer replaces this exe, so a copy of it waits.
            Staged::Installer(setup) => {
                let exe = Self::current_exe().map_err(|e| e.to_string())?;
                let copy = setup.with_file_name(exe.file_name().unwrap_or_default());
                std::fs::copy(&exe, &copy).map_err(|e| format!("couldn't copy {}: {e}", exe.display()))?;
                (copy, Some(setup.clone()))
            }
        };
        let mut cmd = std::process::Command::new(&waiter);
        cmd.arg(FINISH_UPDATE).arg(registry);
        if let Some(setup) = installer {
            cmd.arg(INSTALLER).arg(setup);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        tracing::info!("starting the waiter: {cmd:?}");
        cmd.spawn()
            .map(drop)
            .map_err(|e| format!("couldn't start {}: {e}", waiter.display()))
    }
}

/// The hidden arguments that run `tt-git-gui` as the waiter.
pub const FINISH_UPDATE: &str = "--finish-update";
pub const INSTALLER: &str = "--installer";

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gpui::TestAppContext;
    use tt_git_app::keymap::Command;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::menus::CheckForUpdates;
    use crate::testing::Gui;

    /// Records what the update asked of it.
    #[derive(Default)]
    struct Fake {
        latest: Mutex<Option<Result<String, String>>>,
        install: Option<Result<Install, Refusal>>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(latest: &str, install: Result<Install, Refusal>) -> Arc<Self> {
            Arc::new(Self {
                latest: Mutex::new(Some(Ok(latest.into()))),
                install: Some(install),
                calls: Mutex::default(),
            })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Updater for Fake {
        fn latest(&self) -> Result<String, String> {
            self.calls.lock().unwrap().push("latest".into());
            self.latest.lock().unwrap().clone().expect("latest")
        }
        fn install(&self) -> Result<Install, Refusal> {
            self.install.clone().expect("install")
        }
        fn prepare(&self, tag: &str, _: &Install) -> Result<Staged, String> {
            self.calls.lock().unwrap().push(format!("prepare {tag}"));
            Ok(Staged::Replaced)
        }
        fn start_waiter(&self, _: &Install, _: &Staged, _: &Path) -> Result<(), String> {
            self.calls.lock().unwrap().push("waiter".into());
            Ok(())
        }
    }

    fn folder() -> Install {
        Install::WindowsFolder {
            dir: PathBuf::from("app"),
        }
    }

    fn open(cx: &mut TestAppContext, fake: Arc<Fake>) -> (Gui, Sandbox) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let gui = Gui::open(cx, &sb);
        cx.update(|cx| set_updater(fake, cx));
        (gui, sb)
    }

    fn check_for_updates(gui: &mut Gui) {
        gui.cx.dispatch_action(CheckForUpdates);
        gui.draw();
    }

    fn notice(gui: &Gui) -> String {
        gui.read(|app| app.notice().unwrap_or_default().to_string())
    }

    #[gpui::test]
    fn up_to_date_says_so(cx: &mut TestAppContext) {
        let fake = Fake::new("v0.0.1", Ok(folder()));
        let (mut gui, _sb) = open(cx, fake.clone());
        check_for_updates(&mut gui);
        assert!(gui.cx.has_pending_prompt());
        gui.cx.simulate_prompt_answer("OK");
        gui.draw();
        assert_eq!(fake.calls(), ["latest"]);
        assert!(notice(&gui).ends_with("is up to date"), "{}", notice(&gui));
    }

    #[gpui::test]
    fn a_newer_release_updates_and_restarts(cx: &mut TestAppContext) {
        let fake = Fake::new("v999.0.0", Ok(folder()));
        let (mut gui, _sb) = open(cx, fake.clone());
        check_for_updates(&mut gui);
        assert_eq!(notice(&gui), "tt-git v999.0.0 is available");
        gui.cx.simulate_prompt_answer(UPDATE);
        // Not `draw`, which draws the window it closes.
        gui.cx.run_until_parked();
        assert_eq!(fake.calls(), ["latest", "prepare v999.0.0", "waiter"]);
        assert!(!gui.is_open(), "the window closed to restart");
    }

    #[gpui::test]
    fn later_leaves_it(cx: &mut TestAppContext) {
        let fake = Fake::new("v999.0.0", Ok(folder()));
        let (mut gui, _sb) = open(cx, fake.clone());
        check_for_updates(&mut gui);
        gui.cx.simulate_prompt_answer("Later");
        gui.draw();
        assert_eq!(fake.calls(), ["latest"]);
        assert!(gui.is_open());

        // Not busy any more: it can check again.
        check_for_updates(&mut gui);
        assert!(gui.cx.has_pending_prompt());
    }

    #[gpui::test]
    fn a_copy_that_cant_update_itself_offers_the_release_page(cx: &mut TestAppContext) {
        let refusal = Refusal::Package(PathBuf::from("/usr/bin"));
        let fake = Fake::new("v999.0.0", Err(refusal));
        let (mut gui, _sb) = open(cx, fake.clone());
        check_for_updates(&mut gui);
        gui.cx.simulate_prompt_answer(OPEN_PAGE);
        gui.draw();
        assert_eq!(
            gui.cx.opened_url().as_deref(),
            Some("https://github.com/ThirtyTech/tt-git/releases/tag/v999.0.0")
        );
        assert_eq!(fake.calls(), ["latest"]);
        assert!(gui.is_open());
    }

    #[gpui::test]
    fn a_failed_check_shows_the_error(cx: &mut TestAppContext) {
        let fake = Fake::new("", Ok(folder()));
        *fake.latest.lock().unwrap() = Some(Err("offline".into()));
        let (mut gui, _sb) = open(cx, fake);
        check_for_updates(&mut gui);
        assert!(!gui.cx.has_pending_prompt());
        let error = gui.read(|app| app.error().map(str::to_string)).unwrap_or_default();
        assert_eq!(error, "checking for a tt-git update failed:\noffline");
    }

    #[gpui::test]
    fn the_palettes_update_tt_git_checks_too(cx: &mut TestAppContext) {
        let fake = Fake::new("v0.0.1", Ok(folder()));
        let (mut gui, _sb) = open(cx, fake.clone());
        gui.press(":");
        let offered = gui.read(|app| match app.modal() {
            Some(tt_git_app::modal::Modal::Palette { items, .. }) => {
                items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
            }
            _ => panic!("no palette"),
        });
        assert!(offered.iter().any(|l| l == "Update tt-git"), "{offered:?}");
        gui.press("esc");
        gui.model
            .update(&mut gui.cx, |m, cx| m.send(Action::Command(Command::SelfUpdate), cx));
        gui.draw();
        assert!(gui.cx.has_pending_prompt());
        assert_eq!(fake.calls(), ["latest"]);
    }

    #[gpui::test]
    fn without_an_updater_it_says_it_cant(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        check_for_updates(&mut gui);
        let error = gui.read(|app| app.error().map(str::to_string)).unwrap_or_default();
        assert!(error.contains("can't update itself"), "{error}");
    }

    #[test]
    fn the_background_check_goes_online_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(CHECK_FILE);
        let fake = Fake::new("v2.0.0", Ok(folder()));
        let day = CHECK_EVERY.as_secs();
        let start = 1_000_000;

        assert_eq!(background_check(&*fake, &file, start).as_deref(), Some("v2.0.0"));
        assert_eq!(fake.calls().len(), 1);
        // Within the day: what it found, from the file.
        assert_eq!(
            background_check(&*fake, &file, start + day - 1).as_deref(),
            Some("v2.0.0")
        );
        assert_eq!(fake.calls().len(), 1);
        assert_eq!(background_check(&*fake, &file, start + day).as_deref(), Some("v2.0.0"));
        assert_eq!(fake.calls().len(), 2);

        // Offline: tries again a day later, keeping what it knew.
        *fake.latest.lock().unwrap() = Some(Err("offline".into()));
        assert_eq!(
            background_check(&*fake, &file, start + 2 * day).as_deref(),
            Some("v2.0.0")
        );
        assert_eq!(CheckState::load(&file).checked_at, start + 2 * day);
    }
}
