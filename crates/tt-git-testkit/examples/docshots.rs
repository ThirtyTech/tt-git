//! Renders tt-git against a mock repository for the docs site's screenshots.
//!
//! ```sh
//! cargo run -p tt-git-testkit --example docshots -- docs-site/screens/captures
//! ```
//!
//! Writes one JSON file per screen: every cell's symbol and style, and where
//! each pane was drawn. `docs-site/screens/render.mjs` draws them as images
//! and adds the callouts.

use std::path::{Path, PathBuf};
use std::process::Command;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};
use tt_git_testkit::{NOW, Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

/// 1920×1080 at 10×20 px cells, less the window frame the renderer draws.
const WIDTH: u16 = 190;
const HEIGHT: u16 = 51;

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

const REPO: &str = "lumen-web";

type Person = (&'static str, &'static str);
const ADA: Person = ("Ada Park", "ada@lumen.dev");
const GRACE: Person = ("Grace Okafor", "grace@lumen.dev");
const LINUS: Person = ("Linus Brandt", "linus@lumen.dev");
const MIA: Person = ("Mia Chen", "mia@lumen.dev");
const SAM: Person = ("Sam Rivera", "sam@lumen.dev");

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: docshots <out-dir>"));
    std::fs::create_dir_all(&out).expect("create out dir");

    let sb = Sandbox::named(REPO);
    let mock = Mock::new(&sb);
    mock.build();
    mock.working_changes();

    // Keys for each screen. `text:…` types the rest literally. Shots that
    // change the repo come last, since every shot opens the same one.
    let shots: &[(&str, &[&str])] = &[
        ("overview", &[]),
        ("graph-details", &["j", "space"]),
        ("graph-files", &["j", "enter"]),
        ("graph-filter", &["f"]),
        ("graph-filter-folder", &["f", "tab", "text:src/pages/", "enter"]),
        ("graph-zoom-details", &["+", "space"]),
        ("graph-new-branch", &["j", "n", "text:fix/bell-a11y"]),
        ("graph-new-worktree", &["j", "n", "tab", "text:fix/bell-a11y"]),
        ("graph-merge-confirm", &["j", "j", "j", "j", "j", "M"]),
        ("details-files", &["j", "space", "4", "}"]),
        (
            "details-link",
            &["j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "space"],
        ),
        ("status", &["2"]),
        ("staging", &["2", "j", "j", "j", "enter"]),
        ("staging-lines", &["2", "j", "j", "j", "enter", "a", "j", "v", "j"]),
        ("staging-discard", &["2", "j", "j", "j", "enter", "d"]),
        ("staging-staged", &["2", "j", "j", "j", "j", "enter", "tab"]),
        ("commit", &["2", "c", "text:Show the unread count on the bell"]),
        ("stash-prompt", &["2", "s", "tab", "tab", "text:try a pulse animation"]),
        ("discard-confirm", &["2", "j", "d"]),
        ("branches", &["3"]),
        ("branches-filter", &["3", "/", "text:fpp"]),
        ("branches-new", &["3", "j", "j", "n", "tab", "text:fix/footer-links"]),
        ("branches-delete", &["3", "j", "j", "j", "j", "j", "j", "d"]),
        ("tags", &["3", "]", "j"]),
        ("tags-new", &["3", "]", "T", "tab", "text:v1.6.0"]),
        ("tags-delete", &["3", "]", "j", "d"]),
        ("unmerged", &["3", "]", "]", "j", "j"]),
        ("merged", &["3", "]", "]", "]", "j"]),
        ("stash-tab", &["3", "]", "]", "]", "]"]),
        ("stash-show", &["3", "]", "]", "]", "]", "enter", "+"]),
        ("base-picker", &["3", "b"]),
        ("palette", &["2", ":"]),
        ("palette-search", &[":", "text:sta"]),
        ("help", &["?"]),
        ("help-search", &["?", "text:merge"]),
        ("status-staged", &["2", "j", "space"]),
        ("after-commit", &["2", "c", "text:Bump React to 19.2", "enter"]),
    ];
    for (name, keys) in shots {
        let mut driver = TestDriver::with_size(Config::default(), &sb, WIDTH, HEIGHT);
        press(&mut driver, keys);
        save(&out, name, &driver);
    }

    // The earlier four-pane layout, with details always on screen.
    let four_pane = Config::parse(
        r#"
[layout]
split = "columns"
children = [
  { size = 33, split = "rows", children = [{ pane = "status", size = 40 }, { pane = "branches" }] },
  { split = "rows", children = [{ pane = "graph", size = 65 }, { pane = "details" }] },
]
"#,
    )
    .expect("layout parses");
    let mut driver = TestDriver::with_size(four_pane, &sb, WIDTH, HEIGHT);
    driver.press("j");
    save(&out, "layout-four-pane", &driver);

    // A push two seconds in, in its own repo since it pushes for real.
    // Drawn before the push reports back, so it still shows running.
    let pushing = Sandbox::named(REPO);
    let mock = Mock::new(&pushing);
    mock.build();
    mock.working_changes();
    let mut driver = TestDriver::with_size(Config::default(), &pushing, WIDTH, HEIGHT);
    driver.press_frame("P").tick(9);
    save(&out, "pushing", &driver);

    // A merge stopped on a conflict.
    let conflict = Sandbox::named(REPO);
    let mock = Mock::new(&conflict);
    mock.build();
    mock.conflict();
    let mut driver = TestDriver::with_size(Config::default(), &conflict, WIDTH, HEIGHT);
    press(&mut driver, &["2"]);
    save(&out, "conflict", &driver);

    // Two linked worktrees, in their own repo so the other shots' branches
    // pane doesn't grow a Worktrees tab.
    let trees = Sandbox::named(REPO);
    let mock = Mock::new(&trees);
    mock.build();
    mock.worktrees();
    for (name, keys) in [
        ("worktrees-tab", &["3", "["][..]),
        ("worktree-remove", &["3", "[", "j", "d"][..]),
    ] {
        let mut driver = TestDriver::with_size(Config::default(), &trees, WIDTH, HEIGHT);
        press(&mut driver, keys);
        save(&out, name, &driver);
    }

    // Blame of src/api/client.ts, written by five people over two months,
    // with two lines changed in the working tree. In its own repo, as its
    // commits would change the other shots' graphs.
    let blamed = Sandbox::named(REPO);
    let mock = Mock::new(&blamed);
    mock.build();
    mock.blame_history();
    // From the status pane, on line `line`, then `after`.
    let blame_keys = |line: usize, after: &[&'static str]| {
        let mut keys = vec!["2", "b"];
        keys.extend(std::iter::repeat_n("j", line - 1));
        keys.extend_from_slice(after);
        keys
    };
    for (name, keys) in [
        ("blame", blame_keys(33, &[])),
        ("blame-details", blame_keys(33, &["space"])),
        ("blame-commit", blame_keys(33, &["enter"])),
        ("blame-step-back", blame_keys(46, &["["])),
        ("blame-clock", blame_keys(33, &["t"])),
        ("blame-search", blame_keys(1, &["/", "text:res\\.status", "enter", "n"])),
    ] {
        let mut driver = TestDriver::with_size(Config::default(), &blamed, WIDTH, HEIGHT);
        press(&mut driver, &keys);
        save(&out, name, &driver);
    }

    println!("wrote captures to {}", out.display());
}

fn press(driver: &mut TestDriver, keys: &[&str]) {
    for key in keys {
        match key.strip_prefix("text:") {
            Some(text) => driver.type_text(text),
            None => driver.press(key),
        };
    }
}

/// Writes the screen as `<name>.json`.
fn save(out: &Path, name: &str, driver: &TestDriver) {
    let buf = driver.buffer();
    let mut styles: Vec<Value> = Vec::new();
    let mut rows = Vec::new();
    for y in 0..buf.area.height {
        let mut row = Vec::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let style = json!([color(cell.fg), color(cell.bg), modifiers(cell.modifier)]);
            let idx = styles.iter().position(|s| *s == style).unwrap_or_else(|| {
                styles.push(style);
                styles.len() - 1
            });
            row.push(json!([cell.symbol(), idx]));
        }
        rows.push(Value::Array(row));
    }
    let panes: serde_json::Map<String, Value> = PaneId::ALL
        .iter()
        .filter_map(|p| {
            let r = driver.app().pane_area(*p)?;
            let key = format!("{p:?}").to_lowercase();
            Some((key, json!({"x": r.x, "y": r.y, "w": r.width, "h": r.height})))
        })
        .collect();
    let doc = json!({
        "name": name,
        "width": buf.area.width,
        "height": buf.area.height,
        "styles": styles,
        "rows": rows,
        "panes": panes,
        "text": text(buf),
    });
    let path = out.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string(&doc).expect("json")).expect("write capture");
    println!("{name}");
}

fn text(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

fn color(c: Color) -> String {
    match c {
        Color::Reset => "reset".into(),
        Color::Black => "black".into(),
        Color::Red => "red".into(),
        Color::Green => "green".into(),
        Color::Yellow => "yellow".into(),
        Color::Blue => "blue".into(),
        Color::Magenta => "magenta".into(),
        Color::Cyan => "cyan".into(),
        Color::Gray => "gray".into(),
        Color::DarkGray => "darkgray".into(),
        Color::LightRed => "lightred".into(),
        Color::LightGreen => "lightgreen".into(),
        Color::LightYellow => "lightyellow".into(),
        Color::LightBlue => "lightblue".into(),
        Color::LightMagenta => "lightmagenta".into(),
        Color::LightCyan => "lightcyan".into(),
        Color::White => "white".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("i{i}"),
    }
}

fn modifiers(m: Modifier) -> String {
    [
        (Modifier::BOLD, 'b'),
        (Modifier::DIM, 'd'),
        (Modifier::ITALIC, 'i'),
        (Modifier::UNDERLINED, 'u'),
        (Modifier::REVERSED, 'r'),
        (Modifier::CROSSED_OUT, 'x'),
    ]
    .iter()
    .filter(|(flag, _)| m.contains(*flag))
    .map(|(_, c)| *c)
    .collect()
}

/// Builds the mock project: a small web shop with a few months of history,
/// five authors, feature branches merged in, releases tagged, and branches
/// in every state the branches pane shows.
struct Mock<'a> {
    sb: &'a Sandbox,
    remote: PathBuf,
}

impl<'a> Mock<'a> {
    fn new(sb: &'a Sandbox) -> Self {
        let remote = sb.root().join("remotes").join(format!("{REPO}.git"));
        Self { sb, remote }
    }

    /// Runs git at time `when`, as `who`.
    fn git(&self, when: i64, who: Person, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(self.sb.path());
        self.sb.apply_env(&mut cmd);
        let date = format!("@{when} +0000");
        cmd.env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .env("GIT_AUTHOR_NAME", who.0)
            .env("GIT_AUTHOR_EMAIL", who.1)
            .env("GIT_COMMITTER_NAME", who.0)
            .env("GIT_COMMITTER_EMAIL", who.1);
        let out = cmd.output().expect("spawn git");
        assert!(
            out.status.success() || args[0] == "merge",
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    fn commit(&self, ago: i64, who: Person, files: &[(&str, &str)], msg: &str) {
        for (path, contents) in files {
            self.sb.write(path, contents);
            self.git(NOW - ago, who, &["add", "--", path]);
        }
        self.git(NOW - ago, who, &["commit", "-q", "--allow-empty", "-m", msg]);
    }

    /// A commit that touches one file with a line of its own.
    fn work(&self, ago: i64, who: Person, path: &str, msg: &str) {
        let full = self.sb.path().join(path);
        let mut contents = std::fs::read_to_string(&full).unwrap_or_default();
        contents.push_str(&format!("// {}\n", msg.lines().next().unwrap_or_default()));
        self.commit(ago, who, &[(path, &contents)], msg);
    }

    fn switch(&self, ago: i64, args: &[&str]) {
        let mut all = vec!["switch", "-q"];
        all.extend_from_slice(args);
        self.git(NOW - ago, ADA, &all);
    }

    fn merge(&self, ago: i64, who: Person, branch: &str) {
        let msg = format!("Merge branch '{branch}'");
        self.git(NOW - ago, who, &["merge", "-q", "--no-ff", "-m", &msg, branch]);
    }

    fn tag(&self, ago: i64, who: Person, name: &str, msg: &str) {
        self.git(NOW - ago, who, &["tag", "-a", name, "-m", msg]);
    }

    fn push(&self, ago: i64, args: &[&str]) {
        let mut all = vec!["push", "-q", "origin"];
        all.extend_from_slice(args);
        self.git(NOW - ago, ADA, &all);
    }

    fn build(&self) {
        let d = DAY;
        self.git(
            NOW - 130 * d,
            ADA,
            &["init", "-q", "--bare", "-b", "main", &self.remote.to_string_lossy()],
        );
        self.git(
            NOW - 130 * d,
            ADA,
            &["remote", "add", "origin", &self.remote.to_string_lossy()],
        );

        self.commit(
            120 * d,
            ADA,
            &[
                ("README.md", "# Lumen\n\nThe Lumen storefront.\n"),
                ("package.json", PACKAGE_JSON),
                ("src/main.tsx", "import { App } from './App';\n\nrender(<App />);\n"),
                ("src/App.tsx", "export function App() {\n  return <Layout />;\n}\n"),
            ],
            "Initial commit",
        );
        self.work(118 * d, GRACE, "src/routes.tsx", "Add router and layout");
        self.work(115 * d, ADA, ".github/workflows/ci.yml", "Set up CI");
        self.tag(114 * d, ADA, "v1.0.0", "First public release");
        self.push(114 * d - HOUR, &["-u", "--follow-tags", "main"]);

        self.switch(112 * d, &["-c", "feature/search"]);
        self.work(111 * d, LINUS, "src/api/search.ts", "Add search API client");
        self.switch(110 * d + HOUR, &["main"]);
        self.work(110 * d, GRACE, "src/components/Footer.tsx", "Fix footer links");
        self.switch(109 * d + 2 * HOUR, &["feature/search"]);
        self.work(109 * d + HOUR, LINUS, "src/pages/Search.tsx", "Search results page");
        self.push(109 * d + 30 * MIN, &["feature/search"]);
        self.switch(109 * d + 10 * MIN, &["main"]);
        self.merge(109 * d, ADA, "feature/search");

        self.work(100 * d, MIA, "src/components/ProductCard.tsx", "Add product cards");
        self.tag(99 * d, ADA, "v1.1.0", "Search and product cards");

        self.switch(98 * d, &["-c", "feature/dark-mode"]);
        self.work(97 * d, SAM, "src/styles/tokens.css", "Theme tokens");
        self.switch(96 * d + HOUR, &["main"]);
        self.work(96 * d, ADA, "src/api/client.ts", "Cache API responses");
        self.switch(95 * d + HOUR, &["feature/dark-mode"]);
        self.work(95 * d, SAM, "src/components/ThemeToggle.tsx", "Dark mode toggle");
        self.push(95 * d - 10 * MIN, &["feature/dark-mode"]);
        self.switch(94 * d + HOUR, &["main"]);
        self.merge(94 * d, ADA, "feature/dark-mode");

        self.switch(93 * d + HOUR, &["-c", "fix/login-redirect"]);
        self.work(93 * d, GRACE, "src/pages/Login.tsx", "Keep ?next= after login");
        self.push(93 * d - 10 * MIN, &["fix/login-redirect"]);
        self.switch(92 * d + HOUR, &["main"]);
        self.merge(92 * d, GRACE, "fix/login-redirect");
        self.tag(90 * d, ADA, "v1.2.0", "Dark mode");

        self.work(80 * d, LINUS, "src/pages/Search.tsx", "Paginate search results");
        self.work(75 * d, MIA, "src/pages/Product.tsx", "Product detail page");
        self.tag(70 * d, ADA, "v1.3.0", "Product pages");

        // A fix tagged on a release branch and never merged back.
        self.switch(66 * d, &["-c", "release/1.3"]);
        self.work(65 * d, GRACE, "src/lib/price.ts", "Fix price rounding");
        self.tag(65 * d - HOUR, GRACE, "v1.3.1", "Price rounding fix");
        self.work(65 * d - 3 * HOUR, GRACE, "CHANGELOG.md", "Release notes for 1.3.1");
        self.push(65 * d - 4 * HOUR, &["-u", "release/1.3", "v1.3.1"]);
        self.switch(61 * d, &["main"]);
        // The local copy is a commit behind its upstream: `↓1`.
        self.git(NOW - 61 * d, ADA, &["branch", "-f", "release/1.3", "v1.3.1^{}"]);

        self.switch(60 * d + HOUR, &["-c", "feature/cart"]);
        self.work(60 * d, SAM, "src/store/cart.ts", "Cart store");
        self.work(58 * d, SAM, "src/components/CartDrawer.tsx", "Cart drawer");
        self.switch(57 * d + HOUR, &["main"]);
        self.work(57 * d, ADA, "package.json", "Bump dependencies");
        self.switch(55 * d + HOUR, &["feature/cart"]);
        self.work(55 * d, SAM, "src/components/CheckoutButton.tsx", "Checkout button");
        self.push(55 * d - 10 * MIN, &["feature/cart"]);
        self.switch(54 * d + HOUR, &["main"]);
        self.merge(54 * d, ADA, "feature/cart");
        self.tag(50 * d, ADA, "v1.4.0", "Cart and checkout");
        self.push(50 * d - HOUR, &["--follow-tags", "main"]);

        self.switch(43 * d, &["-c", "feature/wishlist"]);
        self.work(42 * d, MIA, "src/store/wishlist.ts", "Wishlist store");
        self.work(40 * d, MIA, "src/pages/Wishlist.tsx", "Wishlist page");
        self.push(40 * d - 10 * MIN, &["feature/wishlist"]);
        self.switch(38 * d + HOUR, &["main"]);
        self.merge(38 * d, ADA, "feature/wishlist");

        self.switch(36 * d, &["-c", "archive/old-nav"]);
        self.work(35 * d, GRACE, "src/components/Nav.tsx", "Old navigation experiment");
        self.push(35 * d - 10 * MIN, &["archive/old-nav"]);
        self.switch(34 * d, &["main"]);

        self.work(30 * d, GRACE, "src/components/Form.tsx", "Accessibility pass on forms");
        self.work(20 * d, LINUS, "src/pages/Search.tsx", "Search filters");

        // Pushed, then amended and not pushed again: `≠origin`.
        self.switch(19 * d, &["-c", "fix/search-debounce"]);
        self.work(18 * d, LINUS, "src/pages/Search.tsx", "Debounce search input");
        self.push(18 * d - HOUR, &["fix/search-debounce"]);
        self.git(
            NOW - 17 * d,
            LINUS,
            &["commit", "-q", "--amend", "-m", "Debounce search input by 300 ms"],
        );
        self.switch(17 * d - HOUR, &["main"]);

        self.switch(16 * d, &["-c", "spike/graphql"]);
        self.work(15 * d, LINUS, "src/api/graphql.ts", "Try GraphQL for search");
        self.switch(14 * d, &["main"]);

        self.switch(13 * d, &["-c", "chore/deps"]);
        self.work(13 * d - HOUR, ADA, "package.json", "Update dev dependencies");
        // Pushed without setting an upstream: `=origin`.
        self.push(13 * d - 2 * HOUR, &["chore/deps"]);
        self.switch(12 * d + 2 * HOUR, &["main"]);
        self.work(12 * d, ADA, "src/components/ProductGrid.tsx", "Speed up product grid");

        self.switch(11 * d, &["-c", "fix/typo-footer"]);
        self.work(
            11 * d - HOUR,
            MIA,
            "src/components/Footer.tsx",
            "Fix typo in footer\n\n\"Contcat us\" → \"Contact us\".\n\nReported in https://github.com/lumen-dev/lumen-web/issues/42",
        );
        self.push(11 * d - 2 * HOUR, &["-u", "fix/typo-footer"]);
        self.switch(10 * d + HOUR, &["main"]);
        self.merge(10 * d, MIA, "fix/typo-footer");

        self.switch(9 * d, &["-c", "feature/profile-page"]);
        self.work(8 * d, MIA, "src/pages/Profile.tsx", "Profile page skeleton");
        self.work(7 * d, MIA, "src/components/AvatarUpload.tsx", "Avatar upload");
        self.push(7 * d - HOUR, &["-u", "feature/profile-page"]);
        self.switch(6 * d + HOUR, &["main"]);
        self.work(6 * d, MIA, "src/components/LazyImage.tsx", "Fix image lazy loading");

        self.switch(5 * d + 2 * HOUR, &["-c", "feature/checkout-v2"]);
        self.work(5 * d, SAM, "src/pages/Checkout.tsx", "New checkout flow (WIP)");
        self.work(4 * d, SAM, "src/components/AddressForm.tsx", "Address form");
        self.push(4 * d - HOUR, &["feature/checkout-v2"]);
        self.switch(3 * d + 2 * HOUR, &["main"]);
        self.work(3 * d, SAM, "src/pages/Home.tsx", "Update copy on landing page");
        self.commit(
            3 * d - HOUR,
            ADA,
            &[("docs/notes.md", "# Notifications\n\nWhat the bell shows and when.\n")],
            "Notes on notifications",
        );

        self.tag(3 * d - 90 * MIN, ADA, "v1.5.0-beta.1", "Beta: wishlist, faster grid");
        self.push(3 * d - 2 * HOUR, &["--follow-tags", "main"]);
        self.git(NOW - (3 * d - 3 * HOUR), ADA, &["remote", "set-head", "origin", "main"]);
        // Branches only on the remote: merged ones deleted locally, and
        // other people's.
        for b in [
            "feature/search",
            "feature/dark-mode",
            "fix/login-redirect",
            "feature/cart",
            "feature/wishlist",
            "archive/old-nav",
            "feature/checkout-v2",
        ] {
            self.git(NOW - (3 * d - 3 * HOUR), ADA, &["branch", "-q", "-D", b]);
        }
        // Merged and deleted upstream: `gone`.
        self.push(2 * d + 2 * HOUR, &["--delete", "fix/typo-footer"]);
        // A local commit on top of what was pushed: `↑1`.
        self.switch(2 * d + HOUR, &["feature/profile-page"]);
        self.work(2 * d, MIA, "src/pages/Profile.tsx", "Edit display name");
        self.git(NOW - 2 * d + HOUR, ADA, &["fetch", "-q", "--prune", "origin"]);

        // The branch being worked on.
        self.switch(30 * HOUR + 30 * MIN, &["-c", "feature/notifications", "main"]);
        self.commit(
            30 * HOUR,
            ADA,
            &[
                ("src/components/NotificationBell.tsx", BELL),
                ("src/styles/bell.css", BELL_CSS),
            ],
            "Notification bell component",
        );
        self.commit(
            20 * HOUR,
            ADA,
            &[("src/api/notifications.ts", NOTIFICATIONS_API)],
            "Poll for new notifications",
        );
        self.push(20 * HOUR - 10 * MIN, &["-u", "feature/notifications"]);
        let with_read = format!("{NOTIFICATIONS_API}{MARK_ALL_READ}");
        self.commit(
            5 * HOUR,
            ADA,
            &[("src/api/notifications.ts", &with_read)],
            "Mark all as read",
        );

        // Two stashes, older first.
        self.sb
            .write("src/styles/toast.css", ".toast {\n  position: fixed;\n}\n");
        self.git(
            NOW - 2 * HOUR,
            ADA,
            &["stash", "push", "-q", "-u", "-m", "toast styles"],
        );
        self.sb
            .write("src/api/socket.ts", "export const socket = new WebSocket(URL);\n");
        self.git(
            NOW - 40 * MIN,
            ADA,
            &["stash", "push", "-q", "-u", "-m", "try a websocket instead of polling"],
        );
    }

    /// Uncommitted work on `feature/notifications`: staged, unstaged, both,
    /// a rename and a new file.
    fn working_changes(&self) {
        let t = NOW - 10 * MIN;
        // Staged only.
        let api = format!("{NOTIFICATIONS_API}{MARK_ALL_READ}").replace("everyMs = 30_000", "everyMs = 15_000");
        self.sb.write("src/api/notifications.ts", &api);
        self.git(t, ADA, &["add", "src/api/notifications.ts"]);
        // A rename, staged.
        self.git(t, ADA, &["mv", "docs/notes.md", "docs/notifications.md"]);
        // Staged, then changed again.
        self.sb
            .write("src/styles/bell.css", &BELL_CSS.replace("#e5484d", "#d13438"));
        self.git(t, ADA, &["add", "src/styles/bell.css"]);
        self.sb.write(
            "src/styles/bell.css",
            &format!("{}{BELL_CSS_PULSE}", BELL_CSS.replace("#e5484d", "#d13438")),
        );
        // Unstaged, in two hunks.
        self.sb.write("src/components/NotificationBell.tsx", BELL_EDITED);
        self.sb.write(
            "package.json",
            &PACKAGE_JSON.replace("\"react\": \"^19.1.0\"", "\"react\": \"^19.2.0\""),
        );
        // Untracked.
        self.sb.write("src/hooks/useNotifications.ts", USE_NOTIFICATIONS);
    }

    /// A worktree for a new fix branch with work in progress, and a
    /// detached one for reviewing a feature, both under `.worktrees/`.
    fn worktrees(&self) {
        let fix = self.sb.path().join(".worktrees/fix/bell-a11y");
        let review = self.sb.path().join(".worktrees/review-search");
        let (fix_s, review_s) = (fix.to_str().unwrap(), review.to_str().unwrap());
        self.sb
            .git(&["worktree", "add", "-q", "-b", "fix/bell-a11y", fix_s, "main"]);
        self.sb.git(&["worktree", "add", "-q", "--detach", review_s, "main~2"]);
        std::fs::write(self.sb.path().join(".git/info/exclude"), "/.worktrees/\n").unwrap();
        let bell = BELL.replace("<button", "<button aria-label=\"Notifications\"");
        std::fs::write(fix.join("src/components/NotificationBell.tsx"), bell).unwrap();
        std::fs::write(fix.join("src/components/bell.test.tsx"), "// TODO\n").unwrap();
    }

    /// src/api/client.ts built up line by line (see [`CLIENT`]), one commit
    /// per author, then changed in the working tree. The last commit also
    /// touches the notifications client, so it has more than one file.
    fn blame_history(&self) {
        let d = DAY;
        let commits: [(char, i64, Person, &str); 6] = [
            ('A', 60 * d, SAM, "Add API client"),
            ('G', 30 * d, SAM, "Typed API errors"),
            ('D', 21 * d, GRACE, "Retry API requests on 5xx responses"),
            ('C', 14 * d, LINUS, "Search filters"),
            ('F', 7 * d, MIA, "Refresh expired tokens once before failing"),
            (
                'E',
                4 * HOUR,
                ADA,
                "Let callers cancel polling\n\nAdd an AbortSignal option to the client so polling stops when the\nbell unmounts, and return { data } so callers match the old axios shape.",
            ),
        ];
        let order = |tag: char| commits.iter().position(|c| c.0 == tag).expect("known commit");
        for (i, (_, ago, who, msg)) in commits.iter().enumerate() {
            // Each line as of commit `i`: written by then, or what it said
            // before the commit that rewrote it.
            let mut text = String::new();
            for (tag, line, before) in CLIENT {
                let shown = if order(*tag) <= i {
                    Some(*line)
                } else {
                    before.filter(|(t, _)| order(*t) <= i).map(|(_, l)| l)
                };
                if let Some(l) = shown {
                    text.push_str(l);
                    text.push('\n');
                }
            }
            let mut files = vec![("src/api/client.ts", text)];
            if i == commits.len() - 1 {
                let api = format!("{NOTIFICATIONS_API}{MARK_ALL_READ}").replace(
                    "client.get('/notifications', { params })",
                    "client.get('/notifications', { params, signal })",
                );
                files.push(("src/api/notifications.ts", api));
            }
            let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
            self.commit(*ago, *who, &files, msg);
        }
        let mut now: Vec<&str> = CLIENT.iter().map(|(_, l, _)| *l).collect();
        let retry = now.iter().position(|l| l.contains("logger.warn")).expect("retry line");
        now[retry] = "      logger.warn(`retrying ${method} ${path}`, { status: res.status, attempt });";
        now.insert(
            retry + 1,
            "      await new Promise((r) => setTimeout(r, 250 * 2 ** attempt));",
        );
        self.sb.write("src/api/client.ts", &(now.join("\n") + "\n"));
    }

    /// A merge of a branch that changed the bell's markup the same way,
    /// stopped on the conflict, with one other file already resolved.
    fn conflict(&self) {
        self.switch(4 * HOUR, &["-c", "feature/bell-redesign", "feature/notifications~2"]);
        let redesign = BELL.replace(
            "{count > 0 && <span className={styles.badge}>{count}</span>}",
            "{count > 0 && <Badge tone=\"danger\">{count}</Badge>}",
        );
        self.commit(
            3 * HOUR,
            GRACE,
            &[
                ("src/components/NotificationBell.tsx", &redesign),
                ("src/styles/bell.css", &BELL_CSS.replace("#e5484d", "#ff6369")),
            ],
            "Use the shared Badge in the bell",
        );
        self.switch(2 * HOUR, &["feature/notifications"]);
        let mine = BELL.replace(
            "{count > 0 && <span className={styles.badge}>{count}</span>}",
            "{count > 0 && <span className={styles.badge}>{count > 99 ? '99+' : count}</span>}",
        );
        self.commit(
            HOUR,
            ADA,
            &[
                ("src/components/NotificationBell.tsx", &mine),
                ("src/styles/bell.css", &BELL_CSS.replace("#e5484d", "#d13438")),
            ],
            "Cap the badge at 99+",
        );
        self.git(NOW - 30 * MIN, ADA, &["merge", "feature/bell-redesign"]);
        // Resolve the stylesheet (taking neither side), leave the component
        // conflicted.
        self.sb
            .write("src/styles/bell.css", &BELL_CSS.replace("#e5484d", "#e0353b"));
        self.git(NOW - 20 * MIN, ADA, &["add", "src/styles/bell.css"]);
    }
}

const PACKAGE_JSON: &str = r#"{
  "name": "lumen-web",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest"
  },
  "dependencies": {
    "react": "^19.1.0",
    "react-dom": "^19.1.0"
  }
}
"#;

/// src/api/client.ts for the blame shots: each line with the commit (see
/// `Mock::blame_history`) that wrote it, and what it said before if a later
/// commit rewrote it.
type ClientLine = (char, &'static str, Option<(char, &'static str)>);
const CLIENT: &[ClientLine] = &[
    (
        'F',
        "import { getToken, refreshToken } from '../auth/session';",
        Some(('A', "import { getToken } from '../auth/session';")),
    ),
    ('D', "import { logger } from '../lib/logger';", None),
    ('A', "", None),
    ('A', "export type RequestOptions = {", None),
    ('C', "  params?: Record<string, string | number>;", None),
    ('E', "  signal?: AbortSignal;", None),
    ('D', "  retries?: number;", None),
    ('A', "};", None),
    ('A', "", None),
    ('A', "const BASE_URL = import.meta.env.VITE_API_URL ?? '/api';", None),
    ('D', "const DEFAULT_RETRIES = 2;", None),
    ('D', "", None),
    ('G', "export class ApiError extends Error {", None),
    ('G', "  constructor(public status: number, message: string) {", None),
    ('G', "    super(message);", None),
    ('G', "    this.name = 'ApiError';", None),
    ('G', "  }", None),
    ('G', "}", None),
    ('G', "", None),
    (
        'C',
        "function buildUrl(path: string, params?: RequestOptions['params']) {",
        None,
    ),
    ('C', "  const url = new URL(path, BASE_URL);", None),
    (
        'C',
        "  for (const [key, value] of Object.entries(params ?? {})) {",
        None,
    ),
    ('C', "    url.searchParams.set(key, String(value));", None),
    ('C', "  }", None),
    ('C', "  return url.toString();", None),
    ('C', "}", None),
    ('C', "", None),
    (
        'E',
        "async function request<T>(method: string, path: string, opts: RequestOptions = {}): Promise<T> {",
        Some((
            'A',
            "async function request<T>(method: string, path: string, opts: RequestOptions = {}) {",
        )),
    ),
    ('D', "  const retries = opts.retries ?? DEFAULT_RETRIES;", None),
    ('D', "  for (let attempt = 0; ; attempt++) {", None),
    (
        'C',
        "    const res = await fetch(buildUrl(path, opts.params), {",
        Some(('A', "    const res = await fetch(BASE_URL + path, {")),
    ),
    ('A', "      method,", None),
    ('E', "      signal: opts.signal,", None),
    (
        'F',
        "      headers: { Authorization: `Bearer ${await getToken()}` },",
        Some(('A', "      headers: { Authorization: `Bearer ${getToken()}` },")),
    ),
    ('A', "    });", None),
    ('F', "    if (res.status === 401 && attempt === 0) {", None),
    ('F', "      await refreshToken();", None),
    ('F', "      continue;", None),
    ('F', "    }", None),
    ('D', "    if (res.status >= 500 && attempt < retries) {", None),
    (
        'D',
        "      logger.warn(`retrying ${method} ${path} (${res.status})`);",
        None,
    ),
    ('D', "      continue;", None),
    ('D', "    }", None),
    (
        'G',
        "    if (!res.ok) throw new ApiError(res.status, await res.text());",
        Some(('A', "    if (!res.ok) throw new Error(await res.text());")),
    ),
    (
        'E',
        "    return { data: await res.json() } as T;",
        Some(('A', "    return (await res.json()) as T;")),
    ),
    ('D', "  }", None),
    ('A', "}", None),
    ('A', "", None),
    ('A', "export const client = {", None),
    (
        'A',
        "  get: <T>(path: string, opts?: RequestOptions) => request<T>('GET', path, opts),",
        None,
    ),
    (
        'A',
        "  post: <T>(path: string, opts?: RequestOptions) => request<T>('POST', path, opts),",
        None,
    ),
    ('A', "};", None),
];

const BELL: &str = r#"import { useState } from 'react';
import { BellIcon } from './icons';
import styles from '../styles/bell.css';

type Props = {
  count: number;
  onOpen: () => void;
};

export function NotificationBell({ count, onOpen }: Props) {
  const [open, setOpen] = useState(false);

  function toggle() {
    setOpen(!open);
    if (!open) onOpen();
  }

  return (
    <button className={styles.bell} onClick={toggle} aria-label="Notifications">
      <BellIcon />
      {count > 0 && <span className={styles.badge}>{count}</span>}
    </button>
  );
}
"#;

const BELL_EDITED: &str = r#"import { useState } from 'react';
import { BellIcon } from './icons';
import { useNotifications } from '../hooks/useNotifications';
import styles from '../styles/bell.css';

type Props = {
  onOpen: () => void;
};

export function NotificationBell({ onOpen }: Props) {
  const [open, setOpen] = useState(false);
  const { unread } = useNotifications();

  function toggle() {
    setOpen(!open);
    if (!open) onOpen();
  }

  return (
    <button className={styles.bell} onClick={toggle} aria-label={`Notifications (${unread})`}>
      <BellIcon />
      {unread > 0 && <span className={styles.badge}>{unread > 99 ? '99+' : unread}</span>}
    </button>
  );
}
"#;

const BELL_CSS: &str = r#".bell {
  position: relative;
  padding: 6px;
  border-radius: 999px;
}

.badge {
  position: absolute;
  top: -2px;
  right: -2px;
  min-width: 16px;
  background: #e5484d;
  color: white;
  font-size: 11px;
}
"#;

const BELL_CSS_PULSE: &str = r#"
.badge[data-new] {
  animation: pulse 1.2s ease-out 2;
}

@keyframes pulse {
  from { transform: scale(1.4); }
  to { transform: scale(1); }
}
"#;

const NOTIFICATIONS_API: &str = r#"import { client } from './client';

export type Notification = {
  id: string;
  title: string;
  read: boolean;
  createdAt: string;
};

export async function fetchNotifications(since?: string): Promise<Notification[]> {
  const params = since ? { since } : {};
  const res = await client.get('/notifications', { params });
  return res.data;
}

export function pollNotifications(onUpdate: (items: Notification[]) => void, everyMs = 30_000) {
  let last: string | undefined;
  const tick = async () => {
    const items = await fetchNotifications(last);
    if (items.length) {
      last = items[0].createdAt;
      onUpdate(items);
    }
  };
  tick();
  const id = setInterval(tick, everyMs);
  return () => clearInterval(id);
}
"#;

const MARK_ALL_READ: &str = r#"
export async function markAllRead(): Promise<void> {
  await client.post('/notifications/read-all');
}
"#;

const USE_NOTIFICATIONS: &str = r#"import { useEffect, useState } from 'react';
import { pollNotifications, type Notification } from '../api/notifications';

export function useNotifications() {
  const [items, setItems] = useState<Notification[]>([]);
  useEffect(() => pollNotifications((fresh) => setItems((old) => [...fresh, ...old])), []);
  return { items, unread: items.filter((n) => !n.read).length };
}
"#;
