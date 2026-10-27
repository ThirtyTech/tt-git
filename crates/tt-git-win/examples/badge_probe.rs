//! Reproduces the taskbar badge that stays after its window is focused, on
//! real Windows, and reports what happened each time.
//!
//! It starts the debug `tt-git-gui` (which runs as `ThirtyTech.tt-git.dev`,
//! apart from an installed tt-git) on scratch repositories with
//! notifications on, and its own config and state. New commits come from
//! a second clone that commits and pushes, fetched into the app's
//! repository from outside it, so the app counts them on `origin/main`: a
//! toast, and the blue dot on its taskbar button.
//!
//! Each round is checked two ways: what the app logged
//! (`TTGIT_LOG=tt_git_gui=debug`: whether GPUI told it about the
//! activation, and the badge it last set), and what the taskbar shows: the
//! button's pixels, read off the screen, for the dot's colors.
//!
//! Scenarios (`--scenario`):
//!
//! - `single` (the default): one window. Focus a decoy window of the
//!   probe's own, as if you'd gone to another app; new commits; bring the
//!   app's window forward, by clicking its taskbar button (found through
//!   UI Automation) or with `SetForegroundWindow`. With
//!   the window in front, the badge must be gone.
//! - `close-other`: two windows in one app, stacked on one taskbar button.
//!   New commits for the second; focus the first; close the second. With
//!   the first in front and the second gone, the dot must be gone.
//!
//! The debug build writes a backtrace to `borrow-trace.log` for each
//! `RefCell already borrowed` (a window event GPUI dropped because the app
//! was busy); the summary prints the frames that name tt-git, GPUI and the
//! call that waited.
//!
//! It moves the focus and clicks the taskbar while it runs, so leave the
//! mouse and keyboard alone.
//!
//! ```powershell
//! $env:CARGO_TARGET_DIR = 'target/agent'
//! cargo build -p tt-git-gui
//! cargo run -p tt-git-win --example badge_probe -- --rounds 8
//! cargo run -p tt-git-win --example badge_probe -- --mode race --delays 0,5,10,20,40
//! cargo run -p tt-git-win --example badge_probe -- --scenario close-other --rounds 3
//! ```
//!
//! Options:
//!
//! - `--scenario single|close-other`: as above.
//! - `--rounds N`: rounds (per delay, for `single`; default 5).
//! - `--mode settled|race` (`single`): `settled` (the default) waits for
//!   the badge, then the delay, then activates; `race` watches the log
//!   and activates the delay after the app sets the badge, while it's
//!   still sending the toast.
//! - `--delays a,b,...`: milliseconds, as `--mode` says (default 0).
//! - `--activate taskbar|foreground` (`single`): how the window comes
//!   forward (default `taskbar`).
//! - `--no-popup`: badge only, no toasts (`[ui.alerts] popup = false`).
//! - `--gui PATH`: the app to run (default `tt-git-gui.exe` beside the
//!   examples folder this runs from).
//! - `--dir PATH`: where the repositories, config and state go (default a
//!   new folder in `%TEMP%`). It's emptied first. Each check's capture of
//!   the taskbar button is saved there as a PNG.
//! - `--keep`: leave the app running at the end.
//! - `--resend` (`close-other`): at the end of each round, clear the first
//!   window's overlay from the probe (`SetOverlayIcon` with no icon), as
//!   the app could, and check the dot again.

#[cfg(not(windows))]
fn main() {
    eprintln!("badge_probe is for Windows");
}

#[cfg(windows)]
fn main() {
    if let Err(e) = probe::run() {
        eprintln!("badge_probe: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use std::error::Error;
    use std::fs;
    use std::io::{Read as _, Seek as _, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetStockObject, HBRUSH, ReleaseDC, SRCCOPY,
        SelectObject, WHITE_BRUSH,
    };
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationCondition, IUIAutomationElement, TreeScope_Descendants,
    };
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, FindWindowW, GW_OWNER, GetCursorPos,
        GetForegroundWindow, GetMessageW, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_RESTORE, SetCursorPos,
        SetForegroundWindow, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WNDCLASSW,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{BOOL, HSTRING, w};

    type Result<T> = std::result::Result<T, Box<dyn Error>>;

    /// The debug build's AppUserModelID (`tt-git-gui`'s `app_id::APP_ID`).
    const APP_ID: &str = "ThirtyTech.tt-git.dev";

    /// The UI Automation class of an app's button on the taskbar.
    const BUTTON_CLASS: &str = "Taskbar.TaskListButtonAutomationPeer";

    /// How long the app gets to settle after the focus moves.
    const SETTLE: Duration = Duration::from_millis(1500);

    /// How long to wait for the app to badge new commits.
    const BADGE_WAIT: Duration = Duration::from_secs(15);

    /// The badge's colors (`tt-git-gui`'s `Badge::color`), and how far a
    /// pixel may be from one, per channel, to count as it.
    const BLUE: [u8; 3] = [0x1f, 0x6f, 0xeb];
    const RED: [u8; 3] = [0xcf, 0x22, 0x2e];
    const AMBER: [u8; 3] = [0xd2, 0x8a, 0x00];
    const TOLERANCE: i32 = 16;

    /// How many more pixels of a color than the button had without a badge
    /// make a dot.
    const DOT_PIXELS: usize = 6;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Scenario {
        Single,
        CloseOther,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Settled,
        Race,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Activate {
        Taskbar,
        Foreground,
    }

    struct Opts {
        scenario: Scenario,
        rounds: usize,
        mode: Mode,
        delays: Vec<u64>,
        activate: Activate,
        popup: bool,
        gui: PathBuf,
        dir: PathBuf,
        keep: bool,
        resend: bool,
    }

    impl Opts {
        fn parse() -> Result<Self> {
            let exe = std::env::current_exe()?;
            let mut opts = Opts {
                scenario: Scenario::Single,
                rounds: 5,
                mode: Mode::Settled,
                delays: vec![0],
                activate: Activate::Taskbar,
                popup: true,
                // target/<profile>/examples/badge_probe.exe
                gui: exe
                    .parent()
                    .and_then(Path::parent)
                    .map(|p| p.join("tt-git-gui.exe"))
                    .ok_or("no folder for the GUI")?,
                dir: std::env::temp_dir().join(format!("tt-git-badge-probe-{}", std::process::id())),
                keep: false,
                resend: false,
            };
            let mut args = std::env::args().skip(1);
            while let Some(arg) = args.next() {
                let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
                match arg.as_str() {
                    "--scenario" => {
                        opts.scenario = match value()?.as_str() {
                            "single" => Scenario::Single,
                            "close-other" => Scenario::CloseOther,
                            other => return Err(format!("--scenario {other}: single or close-other").into()),
                        }
                    }
                    "--rounds" => opts.rounds = value()?.parse()?,
                    "--mode" => {
                        opts.mode = match value()?.as_str() {
                            "settled" => Mode::Settled,
                            "race" => Mode::Race,
                            other => return Err(format!("--mode {other}: settled or race").into()),
                        }
                    }
                    "--delays" => {
                        opts.delays = value()?
                            .split(',')
                            .map(|d| d.trim().parse())
                            .collect::<std::result::Result<_, _>>()?
                    }
                    "--activate" => {
                        opts.activate = match value()?.as_str() {
                            "taskbar" => Activate::Taskbar,
                            "foreground" => Activate::Foreground,
                            other => return Err(format!("--activate {other}: taskbar or foreground").into()),
                        }
                    }
                    "--no-popup" => opts.popup = false,
                    "--gui" => opts.gui = value()?.into(),
                    "--dir" => opts.dir = value()?.into(),
                    "--keep" => opts.keep = true,
                    "--resend" => opts.resend = true,
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            Ok(opts)
        }
    }

    pub fn run() -> Result<()> {
        let opts = Opts::parse()?;
        if !opts.gui.exists() {
            return Err(format!(
                "{} isn't built: cargo build -p tt-git-gui (with the same CARGO_TARGET_DIR)",
                opts.gui.display()
            )
            .into());
        }
        // SAFETY: called once, first thing, on this thread: physical pixels
        // for UI Automation's rectangles and the screen alike.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }?;
        // SAFETY: as above.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;

        let scratch = Scratch::make(&opts.dir, opts.popup)?;
        println!("scratch: {}", opts.dir.display());
        let decoy = Decoy::open()?;
        let taskbar = Taskbar::new()?;
        let mut log = LogTail::new(scratch.state.join("logs").join("tt-git-gui.log"));
        let mut probe = Probe {
            opts: &opts,
            scratch: &scratch,
            decoy: &decoy,
            taskbar: &taskbar,
            log: &mut log,
            baseline: None,
            captures: 0,
        };
        let result = match opts.scenario {
            Scenario::Single => probe.single(),
            Scenario::CloseOther => probe.close_other(),
        };
        decoy.close();
        if !opts.keep {
            clear_toasts();
        }
        result?;
        traces(&scratch.state.join("logs"));
        Ok(())
    }

    /// The repositories, config and state the app runs with.
    struct Scratch {
        dir: PathBuf,
        /// The repository the app has open.
        repo: PathBuf,
        /// A second one, for a second window.
        repo2: PathBuf,
        /// Another clone, that commits and pushes.
        upstream: PathBuf,
        config: PathBuf,
        state: PathBuf,
    }

    impl Scratch {
        fn make(dir: &Path, popup: bool) -> Result<Self> {
            if dir.exists() {
                fs::remove_dir_all(dir)?;
            }
            fs::create_dir_all(dir)?;
            let origin = dir.join("origin.git");
            let upstream = dir.join("upstream");
            git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"])?;
            git(dir, &["clone", "-q", path(&origin), "upstream"])?;
            for (key, value) in [("user.name", "Badge Probe"), ("user.email", "probe@example.com")] {
                git(&upstream, &["config", key, value])?;
            }
            git(&upstream, &["commit", "-q", "--allow-empty", "-m", "start"])?;
            git(&upstream, &["push", "-q", "origin", "HEAD:main"])?;
            git(dir, &["clone", "-q", path(&origin), "repo"])?;
            git(dir, &["clone", "-q", path(&origin), "repo2"])?;

            let config = dir.join("config");
            let state = dir.join("state");
            fs::create_dir_all(&config)?;
            fs::create_dir_all(&state)?;
            fs::write(
                config.join("config.toml"),
                format!(
                    "[ui]\nnotifications = true\ncheck_for_updates = false\n\n[ui.alerts]\npopup = {popup}\nbadge = true\n"
                ),
            )?;
            Ok(Self {
                dir: dir.to_path_buf(),
                repo: dir.join("repo"),
                repo2: dir.join("repo2"),
                upstream,
                config,
                state,
            })
        }

        /// A new commit on `origin/main`, fetched into `repo` from outside
        /// the app.
        fn new_commit(&self, n: usize, repo: &Path) -> Result<()> {
            git(
                &self.upstream,
                &["commit", "-q", "--allow-empty", "-m", &format!("probe {n}")],
            )?;
            git(&self.upstream, &["push", "-q", "origin", "HEAD:main"])?;
            git(repo, &["fetch", "-q", "origin"])?;
            Ok(())
        }
    }

    fn path(p: &Path) -> &str {
        p.to_str().expect("a UTF-8 scratch path")
    }

    fn git(dir: &Path, args: &[&str]) -> Result<()> {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr)).into());
        }
        Ok(())
    }

    /// The app under test.
    struct App {
        child: Child,
        /// Its windows, in the order of the repositories it was given.
        windows: Vec<isize>,
    }

    impl App {
        /// Starts the app on `repos`, each in a window of its own, and
        /// waits for the windows.
        fn start(gui: &Path, scratch: &Scratch, repos: &[&Path]) -> Result<Self> {
            let child = Command::new(gui)
                .arg("--no-plugins")
                .args(repos)
                .current_dir(&scratch.dir)
                .env("TTGIT_CONFIG_DIR", &scratch.config)
                .env("TTGIT_STATE_DIR", &scratch.state)
                .env("TTGIT_LOG", "info,tt_git_gui=debug")
                .env_remove("TTGIT_SOCKET_PATH")
                .env_remove("TTGIT_REPO_PATH")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let pid = child.id();
            let start = Instant::now();
            // The windows' titles start with their repository's name.
            let titles: Vec<String> = repos
                .iter()
                .map(|r| format!("{} - ", r.file_name().unwrap_or_default().to_string_lossy()))
                .collect();
            loop {
                let open = windows_of(pid);
                let found: Vec<Option<isize>> = titles
                    .iter()
                    .map(|t| open.iter().find(|(_, title)| title.starts_with(t)).map(|(h, _)| *h))
                    .collect();
                if found.iter().all(Option::is_some) {
                    // Let it load the repositories before the first fetch.
                    std::thread::sleep(Duration::from_secs(3));
                    return Ok(Self {
                        child,
                        windows: found.into_iter().flatten().collect(),
                    });
                }
                if start.elapsed() > Duration::from_secs(30) {
                    return Err(format!("the app's windows didn't open in 30 s (have {open:?})").into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        fn stop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// The visible, unowned windows of the process `pid`, with their titles.
    fn windows_of(pid: u32) -> Vec<(isize, String)> {
        struct Search {
            pid: u32,
            found: Vec<(isize, String)>,
        }
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` is the `Search` that EnumWindows was given,
            // alive for the call.
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            let mut pid = 0;
            // SAFETY: plain queries on a window handle EnumWindows gave.
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let mut title = [0u16; 256];
                if pid == search.pid && IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() {
                    let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
                    if n > 0 {
                        search
                            .found
                            .push((hwnd.0 as isize, String::from_utf16_lossy(&title[..n])));
                    }
                }
            }
            true.into()
        }
        let mut search = Search { pid, found: Vec::new() };
        // SAFETY: `search` outlives the call; `each` only reaches it through
        // `lparam`.
        let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut search as *mut Search as isize)) };
        search.found
    }

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut _)
    }

    fn is_foreground(h: isize) -> bool {
        // SAFETY: takes nothing.
        unsafe { GetForegroundWindow() }.0 as isize == h
    }

    fn is_window(h: isize) -> bool {
        // SAFETY: a query on a handle that may be stale, which it reports.
        unsafe { IsWindow(Some(hwnd(h))) }.as_bool()
    }

    /// Makes `h` the foreground window from this process, which Windows
    /// allows while this thread shares input with the current foreground
    /// window's.
    fn force_foreground(h: isize) -> bool {
        // SAFETY: plain calls on window handles and thread ids; attaching
        // input is undone before returning.
        unsafe {
            let window = hwnd(h);
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            }
            let other = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let me = GetCurrentThreadId();
            let attached = other != 0 && other != me && AttachThreadInput(me, other, true).as_bool();
            let done = SetForegroundWindow(window).as_bool();
            if attached {
                let _ = AttachThreadInput(me, other, false);
            }
            done
        }
    }

    /// Waits up to `limit` for `done`.
    fn wait_until(limit: Duration, done: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    fn close_window(h: isize) {
        // SAFETY: posting to a window handle, which fails if it's gone.
        let _ = unsafe { PostMessageW(Some(hwnd(h)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }

    /// A window of the probe's own, to be in when the app shouldn't be.
    struct Decoy {
        hwnd: isize,
    }

    impl Decoy {
        fn open() -> Result<Self> {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                // SAFETY: a window class and window of this thread, and the
                // usual message loop for them.
                unsafe {
                    let instance = match GetModuleHandleW(None) {
                        Ok(i) => i,
                        Err(e) => return tx.send(Err(e.to_string())).unwrap_or(()),
                    };
                    let class = w!("TtGitBadgeProbeDecoy");
                    let wc = WNDCLASSW {
                        lpfnWndProc: Some(decoy_proc),
                        hInstance: instance.into(),
                        lpszClassName: class,
                        hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
                        ..Default::default()
                    };
                    RegisterClassW(&wc);
                    let window = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        w!("badge_probe decoy: the other app you're in"),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        40,
                        40,
                        560,
                        160,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    );
                    match window {
                        Ok(window) => tx.send(Ok(window.0 as isize)).unwrap_or(()),
                        Err(e) => return tx.send(Err(e.to_string())).unwrap_or(()),
                    }
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            });
            let hwnd = rx.recv()??;
            Ok(Self { hwnd })
        }

        fn close(&self) {
            close_window(self.hwnd);
        }
    }

    unsafe extern "system" fn decoy_proc(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_DESTROY {
            // SAFETY: on the window's own thread.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        // SAFETY: the default handling for the window's own message.
        unsafe { DefWindowProcW(window, msg, wparam, lparam) }
    }

    /// The taskbar's buttons, through UI Automation.
    struct Taskbar {
        root: IUIAutomationElement,
        everything: IUIAutomationCondition,
    }

    struct Button {
        name: String,
        element: IUIAutomationElement,
    }

    impl Taskbar {
        fn new() -> Result<Self> {
            // SAFETY: COM is initialized on this thread; the calls take
            // owned or borrowed COM objects and strings.
            unsafe {
                let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                let tray = FindWindowW(w!("Shell_TrayWnd"), None)?;
                let root = uia.ElementFromHandle(tray)?;
                let everything = uia.CreateTrueCondition()?;
                Ok(Self { root, everything })
            }
        }

        /// The first button whose automation id (`Appid: <id>`) passes
        /// `matches`.
        fn button(&self, matches: impl Fn(&str) -> bool) -> Option<Button> {
            // SAFETY: as in `new`.
            unsafe {
                let all = self.root.FindAll(TreeScope_Descendants, &self.everything).ok()?;
                for i in 0..all.Length().ok()? {
                    let element = all.GetElement(i).ok()?;
                    let class = element.CurrentClassName().map(|s| s.to_string()).unwrap_or_default();
                    if class != BUTTON_CLASS {
                        continue;
                    }
                    let id = element.CurrentAutomationId().map(|s| s.to_string()).unwrap_or_default();
                    if matches(&id) {
                        let name = element.CurrentName().map(|s| s.to_string()).unwrap_or_default();
                        return Some(Button { name, element });
                    }
                }
                None
            }
        }

        fn app(&self) -> Option<Button> {
            self.button(|id| id == format!("Appid: {APP_ID}"))
        }

        /// The decoy's button: the probe's own executable.
        fn decoy(&self) -> Option<Button> {
            self.button(|id| id.to_ascii_lowercase().ends_with("badge_probe.exe"))
        }

        /// Clicks `button` with the mouse, as you would: the taskbar's
        /// buttons have no invoke pattern. The pointer goes back after.
        fn click(&self, button: &Button) -> Result<()> {
            // SAFETY: a property read on a live element, then plain input
            // calls with inputs that live through them.
            unsafe {
                let rect = button.element.CurrentBoundingRectangle()?;
                let mut was = POINT::default();
                GetCursorPos(&mut was)?;
                SetCursorPos((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)?;
                let press = |flags| INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dwFlags: flags,
                            ..Default::default()
                        },
                    },
                };
                let inputs = [press(MOUSEEVENTF_LEFTDOWN), press(MOUSEEVENTF_LEFTUP)];
                let sent = SendInput(&inputs, size_of::<INPUT>() as i32);
                std::thread::sleep(Duration::from_millis(50));
                SetCursorPos(was.x, was.y)?;
                if sent != inputs.len() as u32 {
                    return Err("SendInput didn't take the click".into());
                }
            }
            Ok(())
        }
    }

    /// What a capture of the app's taskbar button shows.
    #[derive(Debug, Clone, Copy, Default)]
    struct Pixels {
        blue: usize,
        red: usize,
        amber: usize,
    }

    impl Pixels {
        /// The dot these pixels show, against the button's `baseline`
        /// without a badge: `blue`, `red`, `amber` or `-`.
        fn dot(&self, baseline: Pixels) -> &'static str {
            if self.red >= baseline.red + DOT_PIXELS {
                "red"
            } else if self.amber >= baseline.amber + DOT_PIXELS {
                "amber"
            } else if self.blue >= baseline.blue + DOT_PIXELS {
                "blue"
            } else {
                "-"
            }
        }
    }

    /// The screen's pixels in `rect`, as BGRA rows from the top.
    fn capture(rect: RECT) -> Option<(i32, i32, Vec<u8>)> {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        // SAFETY: the DCs and bitmap are made, used and released here; the
        // buffer holds the 32-bit rows GetDIBits writes for `height` lines.
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(memory, bitmap.into());
            let copied = BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                rect.left,
                rect.top,
                SRCCOPY | CAPTUREBLT,
            );
            SelectObject(memory, old);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    // Negative: top-down rows.
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            if copied.is_err() || lines != height {
                return None;
            }
        }
        Some((width, height, bgra))
    }

    fn near(pixel: &[u8], color: [u8; 3]) -> bool {
        let [r, g, b] = color.map(i32::from);
        let (pb, pg, pr) = (i32::from(pixel[0]), i32::from(pixel[1]), i32::from(pixel[2]));
        (pr - r).abs() <= TOLERANCE && (pg - g).abs() <= TOLERANCE && (pb - b).abs() <= TOLERANCE
    }

    /// Counts the dot's colors on `button`, leaving out its bottom fifth,
    /// where the taskbar draws the running and active marks in the accent
    /// color. Saves the capture as `<name>.png` in `dir`.
    fn read_button(button: &Button, dir: &Path, name: &str) -> Option<Pixels> {
        // SAFETY: a property read on a live element.
        let rect = unsafe { button.element.CurrentBoundingRectangle() }.ok()?;
        let (width, height, bgra) = capture(rect)?;
        let _ = fs::write(dir.join(format!("{name}.png")), png(width as u32, height as u32, &bgra));
        let mut pixels = Pixels::default();
        let rows = (height * 4 / 5) as usize;
        for pixel in bgra.chunks(4).take(rows * width as usize) {
            pixels.blue += usize::from(near(pixel, BLUE));
            pixels.red += usize::from(near(pixel, RED));
            pixels.amber += usize::from(near(pixel, AMBER));
        }
        Some(pixels)
    }

    /// `bgra` as an uncompressed PNG, for looking at a capture.
    fn png(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in bytes {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend(kind);
            out.extend(data);
            let sum = crc(&out[start..]);
            out.extend(sum.to_be_bytes());
        }
        let mut raw = Vec::with_capacity((width * 4 + 1) as usize * height as usize);
        for row in bgra.chunks((width * 4) as usize) {
            raw.push(0);
            for p in row.chunks(4) {
                raw.extend([p[2], p[1], p[0], 255]);
            }
        }
        // zlib with stored (uncompressed) deflate blocks.
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (i, block) in blocks.iter().enumerate() {
            zlib.push(u8::from(i + 1 == blocks.len()));
            let len = block.len() as u16;
            zlib.extend(len.to_le_bytes());
            zlib.extend((!len).to_le_bytes());
            zlib.extend(*block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend(((b << 16) | a).to_be_bytes());
        let mut header = Vec::new();
        header.extend(width.to_be_bytes());
        header.extend(height.to_be_bytes());
        header.extend([8, 6, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// The app's log, read as it grows.
    struct LogTail {
        path: PathBuf,
        pos: u64,
        partial: String,
        lines: Vec<String>,
    }

    impl LogTail {
        fn new(path: PathBuf) -> Self {
            Self {
                path,
                pos: 0,
                partial: String::new(),
                lines: Vec::new(),
            }
        }

        /// Reads what was added since the last read.
        fn read(&mut self) {
            let Ok(mut file) = fs::File::open(&self.path) else {
                return;
            };
            if file.seek(SeekFrom::Start(self.pos)).is_err() {
                return;
            }
            let mut text = String::new();
            if let Ok(n) = file.read_to_string(&mut text) {
                self.pos += n as u64;
            }
            self.partial.push_str(&text);
            while let Some(end) = self.partial.find('\n') {
                let line: String = self.partial.drain(..=end).collect();
                self.lines.push(line.trim_end().to_string());
            }
        }

        /// Waits up to `limit` for a line from `from` on that passes
        /// `found`, reading every millisecond.
        fn wait_for(&mut self, from: usize, limit: Duration, found: impl Fn(&str) -> bool) -> bool {
            let start = Instant::now();
            loop {
                self.read();
                if self.since(from).iter().any(|l| found(l)) {
                    return true;
                }
                if start.elapsed() > limit {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn since(&self, from: usize) -> &[String] {
            &self.lines[from.min(self.lines.len())..]
        }

        /// The badge the app last set: `None` or `Some(NewCommits(1))`.
        fn badge(&self) -> String {
            self.lines
                .iter()
                .rev()
                .find_map(|l| field(l, "taskbar: badge", "badge="))
                .unwrap_or_else(|| "None".into())
        }
    }

    fn badged(line: &str) -> bool {
        line.contains("taskbar: badge") && line.contains("badge=Some(NewCommits")
    }

    /// The value of `key` (up to the next space) on `line`, if it's a
    /// `message` line.
    fn field(line: &str, message: &str, key: &str) -> Option<String> {
        if !line.contains(message) {
            return None;
        }
        let rest = &line[line.find(key)? + key.len()..];
        Some(rest.split(' ').next().unwrap_or("").to_string())
    }

    fn borrow_errors(lines: &[String]) -> usize {
        lines.iter().filter(|l| l.contains("already borrowed")).count()
    }

    /// The longest a call logged as `message` took in `lines`, in ms.
    fn longest(lines: &[String], message: &str) -> Option<f64> {
        lines
            .iter()
            .filter_map(|l| field(l, message, "elapsed="))
            .filter_map(|e| duration_ms(&e))
            .reduce(f64::max)
    }

    /// A `Duration`'s Debug form (`1.2ms`, `350µs`, `2.1s`) in ms.
    fn duration_ms(text: &str) -> Option<f64> {
        let (number, scale) = if let Some(n) = text.strip_suffix("ms") {
            (n, 1.0)
        } else if let Some(n) = text.strip_suffix("µs") {
            (n, 0.001)
        } else if let Some(n) = text.strip_suffix("ns") {
            (n, 0.000_001)
        } else {
            (text.strip_suffix('s')?, 1000.0)
        };
        number.parse::<f64>().ok().map(|n| n * scale)
    }

    fn ms(v: Option<f64>) -> String {
        v.map_or("-".into(), |v| format!("{v:.1}"))
    }

    /// One run of the probe.
    struct Probe<'a> {
        opts: &'a Opts,
        scratch: &'a Scratch,
        decoy: &'a Decoy,
        taskbar: &'a Taskbar,
        log: &'a mut LogTail,
        /// The app's button with no badge on it.
        baseline: Option<Pixels>,
        captures: usize,
    }

    impl Probe<'_> {
        /// Brings `window` forward as `how` says, with `button` its taskbar
        /// button. Falls back to `SetForegroundWindow` when the button
        /// isn't there or doesn't do it.
        fn bring_forward(&self, window: isize, how: Activate, button: Option<Button>) -> &'static str {
            if is_foreground(window) {
                return "already";
            }
            if how == Activate::Taskbar {
                match button.map(|b| self.taskbar.click(&b)) {
                    None => eprintln!("  taskbar: no button for {window:#x}"),
                    Some(Err(e)) => eprintln!("  taskbar: clicking the button for {window:#x} failed: {e}"),
                    Some(Ok(())) if wait_until(Duration::from_secs(2), || is_foreground(window)) => {
                        return "taskbar";
                    }
                    Some(Ok(())) => eprintln!("  taskbar: clicked the button for {window:#x}, it didn't come forward"),
                }
            }
            if force_foreground(window) && wait_until(Duration::from_secs(2), || is_foreground(window)) {
                "foreground"
            } else {
                "failed"
            }
        }

        fn to_decoy(&self) {
            self.bring_forward(self.decoy.hwnd, self.opts.activate, self.taskbar.decoy());
            std::thread::sleep(Duration::from_millis(400));
        }

        /// The dot on the app's button now, with its UI Automation name.
        fn dot(&mut self, label: &str) -> (String, String) {
            self.captures += 1;
            let name = format!("{:03}-{label}", self.captures);
            let Some(button) = self.taskbar.app() else {
                return ("no button".into(), String::new());
            };
            let Some(pixels) = read_button(&button, &self.scratch.dir, &name) else {
                return ("no capture".into(), button.name);
            };
            let baseline = *self.baseline.get_or_insert(pixels);
            (pixels.dot(baseline).to_string(), button.name)
        }

        /// The `single` scenario.
        fn single(&mut self) -> Result<()> {
            let mut app = App::start(&self.opts.gui, self.scratch, &[&self.scratch.repo])?;
            let window = app.windows[0];
            println!("app: pid {}, window {window:#x}", app.child.id());
            self.log.read();
            // The button without a badge, for the pixel counts.
            self.to_decoy();
            let (_, name) = self.dot("baseline");
            println!("taskbar button: {name}");

            let mut rounds = Vec::new();
            let mut n = 0;
            for &delay in &self.opts.delays {
                for _ in 0..self.opts.rounds {
                    n += 1;
                    let round = self.single_round(n, delay, window)?;
                    println!("{}", round.line());
                    rounds.push(round);
                }
            }

            let count = |v: &str| rounds.iter().filter(|r| r.verdict() == v).count();
            println!();
            println!(
                "{} rounds: {} ok, {} STUCK, {} NO-FOCUS; {} with borrow errors; {} where GPUI never reported the activation",
                rounds.len(),
                count("ok"),
                count("STUCK"),
                count("NO-FOCUS"),
                rounds.iter().filter(|r| r.borrow_errors > 0).count(),
                rounds.iter().filter(|r| !r.activation_seen).count(),
            );
            if self.opts.keep {
                println!("left the app running (pid {})", app.child.id());
            } else {
                app.stop();
            }
            Ok(())
        }

        fn single_round(&mut self, n: usize, delay: u64, window: isize) -> Result<SingleRound> {
            self.to_decoy();
            self.log.read();
            let start = self.log.lines.len();

            self.scratch.new_commit(n, &self.scratch.repo)?;
            let (badged_before, dot_badged) = match self.opts.mode {
                Mode::Settled => {
                    let badged = self.log.wait_for(start, BADGE_WAIT, badged);
                    // Explorer draws the overlay a moment after it's set.
                    std::thread::sleep(Duration::from_millis(600));
                    let (dot, _) = self.dot(&format!("r{n}-badged"));
                    std::thread::sleep(Duration::from_millis(delay));
                    (badged, dot)
                }
                Mode::Race => {
                    // As soon as the badge is set the toast goes out: come
                    // forward while it's on its way.
                    let badged = self.log.wait_for(start, BADGE_WAIT, badged);
                    std::thread::sleep(Duration::from_millis(delay));
                    (badged, "-".into())
                }
            };

            self.log.read();
            let before_activation = self.log.lines.len();
            let how = self.bring_forward(window, self.opts.activate, self.taskbar.app());
            std::thread::sleep(SETTLE);
            self.log.read();
            let activation_seen = self
                .log
                .since(before_activation)
                .iter()
                .any(|l| l.contains("taskbar: activation") && l.contains("active=true"));
            let (dot_after, button) = self.dot(&format!("r{n}-after"));
            let lines = self.log.since(start);
            let mut round = SingleRound {
                n,
                delay,
                mode: self.opts.mode,
                badged_before,
                dot_badged,
                how,
                activation_seen,
                os_foreground: is_foreground(window),
                badge: self.log.badge(),
                dot_after,
                borrow_errors: borrow_errors(lines),
                toast_ms: longest(lines, "toast: Show"),
                overlay_ms: longest(lines, "taskbar: SetOverlayIcon"),
                button,
                recovered: None,
            };
            if round.verdict() == "STUCK" {
                self.to_decoy();
                self.bring_forward(window, self.opts.activate, self.taskbar.app());
                std::thread::sleep(SETTLE);
                self.log.read();
                let (dot, _) = self.dot(&format!("r{n}-recovered"));
                round.recovered = Some(self.log.badge() == "None" && dot == "-");
            }
            Ok(round)
        }

        /// The `close-other` scenario.
        fn close_other(&mut self) -> Result<()> {
            let mut stale = 0;
            for n in 1..=self.opts.rounds {
                let repos = [self.scratch.repo.as_path(), self.scratch.repo2.as_path()];
                let mut app = App::start(&self.opts.gui, self.scratch, &repos)?;
                let (first, second) = (app.windows[0], app.windows[1]);
                self.log.read();
                if self.baseline.is_none() {
                    self.to_decoy();
                    let (_, name) = self.dot("baseline");
                    println!("app: pid {}, windows {first:#x} and {second:#x}", app.child.id());
                    println!("taskbar button: {name}");
                }

                // New commits for the second window while you're elsewhere.
                self.to_decoy();
                self.log.read();
                let start = self.log.lines.len();
                self.scratch.new_commit(n, &self.scratch.repo2)?;
                let badged = self.log.wait_for(start, BADGE_WAIT, badged);
                std::thread::sleep(Duration::from_millis(600));
                let (dot_badged, _) = self.dot(&format!("c{n}-second-badged"));

                // In the first window.
                self.bring_forward(first, Activate::Foreground, None);
                std::thread::sleep(SETTLE);
                let (dot_first, _) = self.dot(&format!("c{n}-in-first"));

                // The second window closes.
                close_window(second);
                let closed = wait_until(Duration::from_secs(5), || !is_window(second));
                std::thread::sleep(SETTLE);
                self.log.read();
                let (dot_closed, button) = self.dot(&format!("c{n}-second-closed"));

                // Away and back to the first.
                self.to_decoy();
                self.bring_forward(first, Activate::Foreground, None);
                std::thread::sleep(SETTLE);
                let (dot_back, _) = self.dot(&format!("c{n}-back-in-first"));
                self.log.read();

                // What the first window could do: clear its own overlay
                // again, although it never had one.
                let dot_resent = if self.opts.resend {
                    let cleared = clear_overlay(first);
                    std::thread::sleep(Duration::from_millis(600));
                    let (dot, _) = self.dot(&format!("c{n}-first-resent"));
                    format!("{dot} ({})", if cleared.is_ok() { "cleared" } else { "clear failed" })
                } else {
                    "-".into()
                };

                let verdict = if dot_closed == "-" && dot_back == "-" {
                    "ok"
                } else {
                    stale += 1;
                    "STALE"
                };
                println!(
                    "#{n:<3} second badged {badged:<5} dot {dot_badged:<5} | in first: dot {dot_first:<5} | second closed {closed:<5}: dot {dot_closed:<5} badge {:<20} | away and back: dot {dot_back:<5} | first re-sends no overlay: dot {dot_resent} | borrow-errors {} | {verdict}  [{button}]",
                    self.log.badge(),
                    borrow_errors(self.log.since(start)),
                );
                app.stop();
                wait_until(Duration::from_secs(5), || self.taskbar.app().is_none());
            }
            println!();
            println!("{} rounds: {} STALE", self.opts.rounds, stale);
            Ok(())
        }
    }

    struct SingleRound {
        n: usize,
        delay: u64,
        mode: Mode,
        /// The app badged the commits before the window came forward.
        badged_before: bool,
        /// The dot on the button then.
        dot_badged: String,
        /// How the window came forward.
        how: &'static str,
        /// GPUI told the app about the activation (`taskbar: activation
        /// active=true`).
        activation_seen: bool,
        os_foreground: bool,
        /// The badge the app last set.
        badge: String,
        /// The dot on the button after it came forward.
        dot_after: String,
        borrow_errors: usize,
        toast_ms: Option<f64>,
        overlay_ms: Option<f64>,
        button: String,
        /// For a stuck badge: whether leaving and coming back cleared it.
        recovered: Option<bool>,
    }

    impl SingleRound {
        fn verdict(&self) -> &'static str {
            match (self.os_foreground, self.badge == "None" && self.dot_after == "-") {
                (false, _) => "NO-FOCUS",
                (true, true) => "ok",
                (true, false) => "STUCK",
            }
        }

        fn line(&self) -> String {
            format!(
                "#{:<3} {:<7} delay {:>4}ms  badged {:<5} dot {:<5} | via {:<10} activation-seen {:<5} os-fg {:<5} badge {:<20} dot {:<5} | borrow-errors {:<2} toast {:>5}ms overlay {:>5}ms | {}{}",
                self.n,
                format!("{:?}", self.mode).to_lowercase(),
                self.delay,
                self.badged_before,
                self.dot_badged,
                self.how,
                self.activation_seen,
                self.os_foreground,
                self.badge,
                self.dot_after,
                self.borrow_errors,
                ms(self.toast_ms),
                ms(self.overlay_ms),
                self.verdict(),
                match self.recovered {
                    Some(true) => " (cleared after leaving and coming back)",
                    Some(false) => " (still there after leaving and coming back)",
                    None => "",
                }
            ) + &if self.verdict() == "ok" {
                String::new()
            } else {
                format!("  [{}]", self.button)
            }
        }
    }

    /// Prints each borrow trace's frames that name tt-git, GPUI's window
    /// handling, and the call that waited.
    fn traces(logs: &Path) {
        println!("app log: {}", logs.join("tt-git-gui.log").display());
        let path = logs.join("borrow-trace.log");
        let Ok(text) = fs::read_to_string(&path) else {
            println!("no borrow traces");
            return;
        };
        println!("borrow traces: {}", path.display());
        let keys = [
            "tt_git",
            "gpui::window::",
            "gpui_windows::window::",
            "gpui_windows::events",
            "run_foreground_task",
            "windows::UI::",
            "CoWaitForMultipleHandles",
            "NtUserDispatchMessage",
        ];
        for trace in text.split("=== ").filter(|t| !t.trim().is_empty()) {
            let mut lines = trace.lines();
            println!();
            println!("--- {}", lines.next().unwrap_or(""));
            for line in lines {
                let frame = line.trim();
                let Some((_, function)) = frame.split_once(": ") else {
                    continue;
                };
                if keys.iter().any(|k| function.contains(k))
                    && !frame.starts_with("at ")
                    && !function.contains("borrow_trace")
                {
                    println!("    {}", frame.chars().take(160).collect::<String>());
                }
            }
        }
    }

    /// Takes the overlay off `window`'s taskbar button, from a thread of its
    /// own in a single-threaded apartment, where the taskbar object lives
    /// and goes with the thread. Made on this multithreaded one, releasing
    /// it as the process ends hangs.
    fn clear_overlay(window: isize) -> Result<()> {
        use windows::Win32::System::Com::COINIT_APARTMENTTHREADED;
        std::thread::spawn(move || {
            // SAFETY: first thing on a new thread.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .and_then(|()| tt_git_win::taskbar::set_overlay(window, None))
                .map_err(|e| e.to_string())
        })
        .join()
        .map_err(|_| "the overlay thread panicked")?
        .map_err(Into::into)
    }

    /// Takes the probe's toasts out of the notification center.
    fn clear_toasts() {
        use windows::UI::Notifications::ToastNotificationManager;
        let cleared = ToastNotificationManager::History().and_then(|h| h.ClearWithId(&HSTRING::from(APP_ID)));
        if let Err(e) = cleared {
            eprintln!("couldn't clear the toasts: {e}");
        }
    }
}
