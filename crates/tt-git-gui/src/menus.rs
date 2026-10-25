//! The app's menus: tt-git's commands, grouped as the help popup and the
//! palette group them, each showing the key tt-git's keymap binds to it.
//!
//! tt-git's keymap stays the only place keys are bound. A menu item carries
//! a [`RunCommand`] (one action for every command), and the shell runs it
//! by sending the app `Action::Command`, which is what a key bound to the
//! command does. The key is part of the item's text, worked out from the
//! keymap in the focused pane's context, so it follows a user's rebinds;
//! no GPUI key binding is made for it.
//!
//! On macOS the menus are the native menu bar ([`install`] through
//! `cx.set_menus`). GPUI keeps the menus on Windows and Linux without
//! showing them, so the window draws them itself in its title bar
//! ([`bar`]).

use gpui::{Action as _, App, Global, Menu, MenuItem};
use tt_git_app::keymap::{Command, Context};

pub mod bar;
pub mod icon;

/// Runs a tt-git command in the window it's sent to, as its key would.
#[derive(Debug, Clone, PartialEq, gpui::Action)]
#[action(namespace = tt_git, no_json)]
pub struct RunCommand(pub Command);

gpui::actions!(
    tt_git,
    [
        /// Opens the About dialog.
        About,
        /// Quits every window, as the quit key does in each.
        Quit
    ]
);

/// A menu: its name and its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSpec {
    pub name: String,
    pub items: Vec<ItemSpec>,
}

/// One entry in a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSpec {
    Separator,
    Item {
        /// What the item says, without its key.
        label: String,
        /// The key bound to it in the focused pane's context, as the help
        /// bar shows it.
        key: Option<String>,
        action: MenuAction,
        enabled: bool,
    },
}

/// What a menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Command(Command),
    About,
    Quit,
}

impl MenuAction {
    pub fn boxed(self) -> Box<dyn gpui::Action> {
        match self {
            MenuAction::Command(cmd) => RunCommand(cmd).boxed_clone(),
            MenuAction::About => About.boxed_clone(),
            MenuAction::Quit => Quit.boxed_clone(),
        }
    }
}

/// The repository-wide git commands, as the palette offers them from any
/// pane.
const REPOSITORY: &[&[Command]] = &[
    &[
        Command::Fetch,
        Command::PruneGone,
        Command::Pull,
        Command::Push,
        Command::Refresh,
    ],
    &[Command::Continue, Command::Abort, Command::Undo],
    &[Command::Agent],
    &[Command::OpenRepository],
];

/// Moving between panes and what the window shows.
const VIEW: &[&[Command]] = &[
    &[
        Command::FocusGraph,
        Command::FocusStatus,
        Command::FocusBranches,
        Command::FocusDetails,
    ],
    &[Command::FocusNext, Command::FocusPrev],
    &[Command::Zoom],
    &[Command::ZoomIn, Command::ZoomOut, Command::ResetZoom],
    &[Command::Palette, Command::Help],
];

/// The app's settings and tools, in the app menu between About and Quit.
const APP: &[Command] = &[Command::Theme, Command::PluginLogs, Command::SelfUpdate];

/// The menus for `app` as it is now: the app menu, Repository, View, and a
/// menu named after the focused pane listing its commands (the help
/// popup's first section). While a dialog, the help popup or an error is
/// open, keys go to it rather than to commands, so the command items are
/// disabled; About and Quit stay enabled.
pub fn build(app: &tt_git_app::App) -> Vec<MenuSpec> {
    let context = app.keymap_context();
    let keymap = app.keymap();
    let enabled = app.modal().is_none() && app.error().is_none() && !app.shows_help();
    let item = |cmd: Command| ItemSpec::Item {
        label: sentence_case(cmd.label()),
        key: keymap.display_key_text(context, cmd).map(str::to_string),
        action: MenuAction::Command(cmd),
        enabled,
    };
    let groups = |groups: &[&[Command]]| -> Vec<ItemSpec> {
        let mut items = Vec::new();
        for (i, group) in groups.iter().enumerate() {
            if i > 0 {
                items.push(ItemSpec::Separator);
            }
            items.extend(group.iter().map(|c| item(*c)));
        }
        items
    };

    let mut app_items = vec![
        ItemSpec::Item {
            label: "About tt-git".into(),
            key: None,
            action: MenuAction::About,
            enabled: true,
        },
        ItemSpec::Separator,
    ];
    // Update tt-git only where `tt-git update` can update this front end.
    app_items.extend(
        APP.iter()
            .filter(|c| app.self_updates() || **c != Command::SelfUpdate)
            .map(|c| item(*c)),
    );
    app_items.push(ItemSpec::Separator);
    app_items.push(ItemSpec::Item {
        label: "Quit".into(),
        key: keymap.display_key_text(context, Command::Quit).map(str::to_string),
        action: MenuAction::Quit,
        enabled: true,
    });

    vec![
        MenuSpec {
            name: "tt-git".into(),
            items: app_items,
        },
        MenuSpec {
            name: "Repository".into(),
            items: groups(REPOSITORY),
        },
        MenuSpec {
            name: "View".into(),
            items: groups(VIEW),
        },
        MenuSpec {
            name: pane_menu_name(context).into(),
            items: app.focused_commands().iter().map(|c| item(*c)).collect(),
        },
    ]
}

/// The pane menu's name: the context its keys come from.
fn pane_menu_name(context: Context) -> &'static str {
    match context {
        Context::Global | Context::Graph => "Graph",
        Context::Status => "Status",
        Context::Branches => "Branches",
        Context::Details => "Details",
        Context::Stage => "Stage",
        Context::Stash => "Stash",
        Context::Worktrees => "Worktrees",
        Context::Blame => "Blame",
    }
}

/// `label` with its first letter capitalised, as the palette shows it.
fn sentence_case(label: &str) -> String {
    let mut out = label.to_string();
    if let Some(first) = out.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    out
}

/// An item's text in a native menu, which has no place for a key the
/// keymap (rather than GPUI) binds: the key follows the label.
pub fn native_label(label: &str, key: Option<&str>) -> String {
    match key {
        Some(key) => format!("{label}  ({key})"),
        None => label.to_string(),
    }
}

/// `specs` as GPUI menus, for `cx.set_menus`.
pub fn native(specs: &[MenuSpec]) -> Vec<Menu> {
    specs
        .iter()
        .map(|menu| {
            Menu::new(menu.name.clone()).items(menu.items.iter().map(|item| match item {
                ItemSpec::Separator => MenuItem::separator(),
                ItemSpec::Item {
                    label,
                    key,
                    action,
                    enabled,
                } => {
                    let name = native_label(label, key.as_deref());
                    let item = match *action {
                        MenuAction::Command(cmd) => MenuItem::action(name, RunCommand(cmd)),
                        MenuAction::About => MenuItem::action(name, About),
                        MenuAction::Quit => MenuItem::action(name, Quit),
                    };
                    item.disabled(!enabled)
                }
            }))
        })
        .collect()
}

/// The menus last given to the platform.
struct Installed(Vec<MenuSpec>);

impl Global for Installed {}

/// Gives the platform `specs` as its menus, unless they're the ones it has.
/// The menus are the app's, not a window's: the active window installs its
/// own.
pub fn install(specs: &[MenuSpec], cx: &mut App) {
    if cx.try_global::<Installed>().is_some_and(|i| i.0 == specs) {
        return;
    }
    cx.set_menus(native(specs));
    cx.set_global(Installed(specs.to_vec()));
}

/// Registers what the menus need app-wide: [`Quit`], which works with no
/// window focused.
pub fn init(cx: &mut App) {
    // Deferred: it's dispatched while the active window is being updated,
    // and each window is updated in turn.
    cx.on_action(|_: &Quit, cx| cx.defer(quit_all));
}

/// Sends every window the quit command, so each closes as its quit key
/// would close it; GPUI quits once the last one has. With no tt-git
/// window, quits at once.
fn quit_all(cx: &mut App) {
    let shells: Vec<_> = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<gpui::component::Root>())
        .filter_map(|root| {
            root.update(cx, |root, _, _| {
                root.view().clone().downcast::<crate::shell::Shell>().ok()
            })
            .ok()
            .flatten()
        })
        .collect();
    if shells.is_empty() {
        cx.quit();
        return;
    }
    for shell in shells {
        let model = shell.read(cx).model().clone();
        model.update(cx, |m, cx| m.send(tt_git_app::Action::Command(Command::Quit), cx));
    }
}

#[cfg(test)]
mod tests {
    use gpui::{OwnedMenu, OwnedMenuItem, TestAppContext};
    use tt_git_app::Config;
    use tt_git_app::keymap::Command;
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.commit_file("b.txt", "b", "second");
        sb
    }

    fn menus(gui: &mut Gui) -> Vec<OwnedMenu> {
        gui.cx.update(|_, cx| cx.get_menus()).expect("menus installed")
    }

    fn names(gui: &mut Gui) -> Vec<String> {
        menus(gui).iter().map(|m| m.name.to_string()).collect()
    }

    /// The item `name` (its text, key included) in the menu `menu`:
    /// whether it's disabled, and its action.
    fn item(gui: &mut Gui, menu: &str, name: &str) -> Option<(bool, Box<dyn gpui::Action>)> {
        menus(gui)
            .into_iter()
            .find(|m| m.name.as_ref() == menu)?
            .items
            .into_iter()
            .find_map(|i| match i {
                OwnedMenuItem::Action {
                    name: n,
                    action,
                    disabled,
                    ..
                } if n == name => Some((disabled, action)),
                _ => None,
            })
    }

    #[gpui::test]
    fn the_menus_list_commands_with_their_keys(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        assert_eq!(names(&mut gui), ["tt-git", "Repository", "View", "Graph"]);
        assert!(item(&mut gui, "tt-git", "About tt-git").is_some());
        assert!(item(&mut gui, "tt-git", "Quit  (q)").is_some());
        assert!(item(&mut gui, "Graph", "Checkout  (o)").is_some());
        assert!(item(&mut gui, "View", "Status  (2)").is_some());
        assert!(item(&mut gui, "Repository", "Push  (P)").is_some());
    }

    #[gpui::test]
    fn a_menu_item_runs_its_command(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        let (disabled, action) = item(&mut gui, "View", "Status  (2)").expect("item");
        assert!(!disabled);
        // As the platform sends it: to the window's focused element.
        gui.cx.update(|window, cx| window.dispatch_action(action, cx));
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status);

        gui.cx.dispatch_action(RunCommand(Command::Help));
        gui.wait_idle();
        assert!(gui.read(|app| app.shows_help()));
    }

    #[gpui::test]
    fn the_pane_menu_follows_the_focus(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("tab");
        assert_eq!(names(&mut gui), ["tt-git", "Repository", "View", "Status"]);
        assert!(item(&mut gui, "Status", "Stage/unstage  (space)").is_some());
        gui.press("tab");
        assert_eq!(names(&mut gui)[3], "Branches");
        assert!(item(&mut gui, "Branches", "Checkout  (enter)").is_some());
    }

    #[gpui::test]
    fn labels_follow_a_rebind(cx: &mut TestAppContext) {
        let sb = sandbox();
        let config = Config::parse("[keys.graph]\ncheckout = \"ctrl+o\"\n[keys.global]\nquit = \"x\"\n").unwrap();
        let mut gui = Gui::with_config(cx, &sb, config);
        assert!(item(&mut gui, "Graph", "Checkout  (ctrl+o)").is_some());
        assert!(item(&mut gui, "Graph", "Checkout  (o)").is_none());
        assert!(item(&mut gui, "tt-git", "Quit  (x)").is_some());
    }

    #[gpui::test]
    fn commands_are_disabled_while_a_dialog_has_the_keys(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("?");
        let (disabled, _) = item(&mut gui, "Graph", "Checkout  (o)").expect("item");
        assert!(disabled, "keys go to the help popup");
        let (disabled, _) = item(&mut gui, "tt-git", "Quit  (q)").expect("item");
        assert!(!disabled);
        gui.press("esc");
        let (disabled, _) = item(&mut gui, "Graph", "Checkout  (o)").expect("item");
        assert!(!disabled);
    }

    #[gpui::test]
    fn update_tt_git_points_at_the_release_instead(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        // `tt-git update` installs the TUI's archive, not the app.
        assert!(!gui.read(|app| app.self_updates()));
        let app_menu: Vec<String> = menus(&mut gui)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                OwnedMenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect();
        assert!(app_menu.iter().any(|n| n.starts_with("Theme")), "{app_menu:?}");
        assert!(!app_menu.iter().any(|n| n.starts_with("Update tt-git")), "{app_menu:?}");

        gui.press(":");
        let offered = gui.read(|app| match app.modal() {
            Some(tt_git_app::modal::Modal::Palette { items, .. }) => {
                items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
            }
            _ => panic!("no palette"),
        });
        assert!(offered.iter().any(|l| l == "Theme"), "{offered:?}");
        assert!(!offered.iter().any(|l| l == "Update tt-git"), "{offered:?}");
        gui.press("esc");

        // A plugin or a key can still run it: it says where to update.
        gui.cx.dispatch_action(RunCommand(Command::SelfUpdate));
        gui.wait_idle();
        let notice = gui.read(|app| app.notice().unwrap_or_default().to_string());
        assert!(notice.starts_with("update the desktop app by downloading"), "{notice}");
        assert!(notice.ends_with("/releases/latest"), "{notice}");
        assert_eq!(gui.read(|app| app.error().map(str::to_string)), None);
    }

    #[gpui::test]
    fn quit_closes_the_window(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.cx.dispatch_action(Quit);
        gui.draw();
        assert!(!gui.is_open());
    }
}
