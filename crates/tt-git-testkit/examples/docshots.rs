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

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};
use tt_git_testkit::mock::{Mock, REPO};
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;
use tt_git_tui::themes::ThemeOptions;

/// 1920×1080 at 10×20 px cells, less the window frame the renderer draws.
const WIDTH: u16 = 190;
const HEIGHT: u16 = 51;

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
        ("graph-reset", &["j", "j", "j", "X"]),
        ("graph-reset-hard", &["j", "j", "j", "X", "text:hard", "enter"]),
        (
            "graph-cherry-pick-range",
            &["j", "j", "j", "j", "j", "v", "j", "j", "C"],
        ),
        (
            "agent-dialog",
            &["j", "A", "tab", "tab", "text:start from the bell icon"],
        ),
        (
            "agent-hunk-staging",
            &["2", "j", "j", "j", "enter", "a", "j", "v", "j", "A", "tab"],
        ),
        (
            "agent-hunk-details",
            &["j", "space", "4", "+", "A", "text:why poll every 30 seconds?"],
        ),
        ("details-files", &["j", "space", "4", "}"]),
        ("details-search", &["j", "enter", "/", "text:items", "enter", "n"]),
        (
            "details-link",
            &["j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "j", "space"],
        ),
        ("status", &["2"]),
        ("status-zoom", &["2", "+"]),
        ("staging", &["2", "j", "j", "j", "enter"]),
        ("staging-lines", &["2", "j", "j", "j", "enter", "a", "j", "v", "j"]),
        ("staging-discard", &["2", "j", "j", "j", "enter", "d"]),
        ("staging-staged", &["2", "j", "j", "j", "j", "enter", "tab"]),
        ("commit", &["2", "c", "text:Show the unread count on the bell"]),
        ("stash-prompt", &["2", "s", "tab", "tab", "text:try a pulse animation"]),
        ("discard-confirm", &["2", "j", "d"]),
        ("discard-all", &["2", "D"]),
        ("ignore-picker", &["2", "G", "k", "i"]),
        ("branches", &["3"]),
        ("branches-filter", &["3", "/", "text:fpp"]),
        ("branches-new", &["3", "j", "j", "n", "tab", "text:fix/footer-links"]),
        ("branches-delete", &["3", "j", "j", "j", "j", "j", "j", "d"]),
        ("branches-prune-gone", &["3", "D"]),
        ("branches-push-which", &["3", "/", "text:fpp", "enter", "P"]),
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
        ("help-about", &["?", "tab"]),
        ("about", &[":", "text:about", "enter"]),
        ("status-staged", &["2", "j", "space"]),
        ("after-commit", &["2", "c", "text:Bump React to 19.2", "enter"]),
    ];
    // Status as a folder tree, then discarding a folder in it. Before the
    // shots below change the files.
    let tree = Config {
        status: tt_git_tui::config::StatusConfig { tree: true },
        ..Config::default()
    };
    for (name, keys) in [
        ("status-tree", &["2"][..]),
        ("status-tree-discard", &["2", "G", "k", "k", "d"]),
    ] {
        let mut driver = TestDriver::with_size(tree.clone(), &sb, WIDTH, HEIGHT);
        press(&mut driver, keys);
        save(&out, name, &driver);
    }
    for (name, keys) in shots {
        let mut driver = TestDriver::with_size(Config::default(), &sb, WIDTH, HEIGHT);
        press(&mut driver, keys);
        save(&out, name, &driver);
    }

    // The theme picker, four down the list so Catppuccin Mocha shows behind
    // it, and a light theme on its own background. Only the built-in
    // themes, so the user's own and Omarchy's don't show up.
    let mut driver = TestDriver::with_size(Config::default(), &sb, WIDTH, HEIGHT);
    driver.app_mut().enable_themes(ThemeOptions::default());
    press(
        &mut driver,
        &[":", "text:theme", "enter", "down", "down", "down", "down"],
    );
    save(&out, "theme-picker", &driver);
    let light = Config {
        theme: Some("catppuccin-latte".into()),
        theme_background: true,
        ..Config::default()
    };
    let mut driver = TestDriver::with_size(light, &sb, WIDTH, HEIGHT);
    driver.app_mut().enable_themes(ThemeOptions::default());
    driver.render();
    save(&out, "theme-light", &driver);

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
        // Checking out a branch the fix worktree has opens that worktree.
        ("worktree-switch", &["3", "/", "text:bell", "enter", "enter"][..]),
    ] {
        let mut driver = TestDriver::with_size(Config::default(), &trees, WIDTH, HEIGHT);
        press(&mut driver, keys);
        save(&out, name, &driver);
    }

    // The message filter's presets are the repo's most used subject
    // prefixes, and the mock's subjects have none, so this one adds some on
    // top, in its own repo. The shot only shows the prompt.
    let prefixed = Sandbox::named(REPO);
    let mock = Mock::new(&prefixed);
    mock.build();
    mock.conventional_commits();
    let mut driver = TestDriver::with_size(Config::default(), &prefixed, WIDTH, HEIGHT);
    press(&mut driver, &["/", "tab", "tab"]);
    save(&out, "graph-filter-message", &driver);
    // ctrl+t twice searches the diff, git log -G.
    let mut driver = TestDriver::with_size(Config::default(), &prefixed, WIDTH, HEIGHT);
    press(&mut driver, &["/", "ctrl+t", "ctrl+t", "text:notification", "enter"]);
    save(&out, "graph-filter-diff", &driver);

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
