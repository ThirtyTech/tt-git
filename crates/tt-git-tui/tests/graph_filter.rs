use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::filter::FilterTab;
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::PaneId;

const ALICE: &str = "Alice <alice@example.com>";

/// Commits a change to `file` as `author` (`Name <email>`).
fn commit_as(sb: &Sandbox, author: &str, file: &str, msg: &str) {
    sb.write(file, &format!("{msg}\n"));
    sb.git(&["add", "--", file]);
    sb.tick();
    sb.git(&["commit", "-q", &format!("--author={author}"), "-m", msg]);
}

/// Alice and Test User both commit on main and on a side branch.
fn two_authors() -> Sandbox {
    let sb = Sandbox::new();
    commit_as(&sb, ALICE, "src/lib.rs", "alice lib");
    sb.commit_file("README.md", "readme\n", "test readme");
    sb.checkout_new("side");
    commit_as(&sb, ALICE, "docs/guide.md", "alice guide");
    sb.commit_file("src/lib.rs", "changed\n", "test lib");
    sb.checkout("main");
    sb
}

fn summaries(t: &TestDriver) -> Vec<String> {
    t.app()
        .graph()
        .commits()
        .iter()
        .map(|c| c.summary.to_string())
        .collect()
}

#[test]
fn filter_panel_lists_authors_and_files() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("f");
    assert!(matches!(t.app().modal(), Some(Modal::Filter(_))));
    insta::assert_snapshot!("filter_panel_authors", t.screen());
    t.press("tab");
    insta::assert_snapshot!("filter_panel_files", t.screen());
    t.press("esc");
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().graph().commits().len(), 4, "cancelling leaves the graph alone");
}

#[test]
fn filter_graph_by_author_then_file_then_clear() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    assert_eq!(t.app().graph().commits().len(), 4);

    t.press("f").type_text("alice").press("enter");
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · author Alice]"));

    // Adding a file keeps the author.
    t.press("f").press("tab").type_text("src/lib").press("enter");
    assert_eq!(summaries(&t), ["alice lib"]);
    assert!(
        t.pane_title(PaneId::Graph)
            .contains("Graph [all · author Alice · file src/lib.rs]")
    );

    // "Any author" leads the author list once one is picked.
    t.press("f").press("enter");
    assert_eq!(summaries(&t), ["test lib", "alice lib"]);
    // Path filters keep the history connected.
    let commits = t.app().graph().commits();
    assert_eq!(*commits[0].parents, [commits[1].oid]);

    // The filter applies to the current branch too.
    t.press("a");
    assert_eq!(summaries(&t), ["alice lib"]);
    t.press("a");

    // esc (with details closed) shows everything again.
    t.press("esc");
    assert_eq!(t.app().graph().commits().len(), 4);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all] "));
}

#[test]
fn filtering_to_nothing_says_so() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("f").press("tab").type_text("no/such/file").press("enter");
    assert!(t.app().graph().commits().is_empty());
    t.assert_pane_contains(PaneId::Graph, "no commits match the filter");
    // With details open, esc closes them first and keeps the filter.
    t.press("space").press("esc");
    assert!(!t.app().shows(PaneId::Details));
    assert!(t.app().graph().is_filtered());
    t.press("esc");
    assert!(!t.app().graph().is_filtered());
}

#[test]
fn filter_panel_takes_clicks() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    let tab = |t: &TestDriver| match t.app().modal() {
        Some(Modal::Filter(panel)) => Some(panel.tab()),
        _ => None,
    };

    t.press("f");
    t.click_text("File: any");
    assert_eq!(tab(&t), Some(FilterTab::File));
    t.click_text("Author: any");
    assert_eq!(tab(&t), Some(FilterTab::Author));

    // Clicking outside the panel closes it.
    t.click(0, 0, false);
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().graph().commits().len(), 4);

    // Clicking a choice applies it.
    t.press("f");
    t.click_text("Alice <alice@example.com>");
    assert!(t.app().modal().is_none());
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
    assert_eq!(t.app().focus(), PaneId::Graph);

    t.press("f").press("tab");
    t.click_text("src/lib.rs");
    assert_eq!(summaries(&t), ["alice lib"]);

    // The wheel moves the selection: down past the end, then back to
    // "Any file" at the top.
    t.press("f").press("tab");
    let (x, y) = t.find_text("Any file").unwrap();
    t.scroll(x, y, true).scroll(x, y, false).scroll(x, y, false);
    t.press("enter");
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
}

/// Commits in `app/` and `lib/`, then uncommitted changes in both.
fn two_folders() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("app/main.rs", "1\n", "app one");
    sb.commit_file("lib/lib.rs", "1\n", "lib one");
    sb.commit_file("app/main.rs", "2\n", "app two");
    sb.write("app/main.rs", "3\n").write("lib/lib.rs", "2\n");
    sb.write("app/new.rs", "n\n");
    sb.git(&["add", "lib/lib.rs"]);
    sb
}

fn status_paths(t: &TestDriver) -> Vec<String> {
    t.app().status().entries().iter().map(|e| e.path.clone()).collect()
}

#[test]
fn dot_narrows_graph_and_status_to_the_start_folder() {
    let sb = two_folders();
    let mut t = TestDriver::at(Config::default(), &sb, &sb.path().join("app"), 140, 24);
    assert_eq!(t.app().graph().commits().len(), 3, "starts unfiltered by default");
    assert_eq!(status_paths(&t).len(), 3);

    t.press(".");
    assert_eq!(summaries(&t), ["app two", "app one"]);
    let commits = t.app().graph().commits();
    assert_eq!(*commits[0].parents, [commits[1].oid], "history stays connected");
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · folder app/]"));
    assert_eq!(status_paths(&t), ["app/main.rs", "app/new.rs"]);
    assert!(
        t.pane_title(PaneId::Status)
            .contains("· in app/ · +1 outside (1 staged)"),
        "{}",
        t.pane_title(PaneId::Status)
    );
    insta::assert_snapshot!("folder_filter", t.screen());

    // From Status too, and back again.
    t.press("2").press(".");
    assert_eq!(t.app().graph().commits().len(), 3);
    assert_eq!(status_paths(&t).len(), 3);
    assert!(!t.pane_title(PaneId::Status).contains("in app/"));

    // esc in the graph clears it, Status included.
    t.press("1").press(".").press("esc");
    assert_eq!(t.app().graph().commits().len(), 3);
    assert_eq!(status_paths(&t).len(), 3);
}

#[test]
fn filter_to_cwd_starts_narrowed() {
    let sb = two_folders();
    let mut config = Config::default();
    config.graph.filter_to_cwd = true;
    let t = TestDriver::at(config.clone(), &sb, &sb.path().join("app"), 100, 30);
    assert_eq!(summaries(&t), ["app two", "app one"]);
    assert_eq!(status_paths(&t), ["app/main.rs", "app/new.rs"]);

    // At the root there's no folder, so nothing is narrowed and . says why.
    let mut t = TestDriver::with_size(config, &sb, 160, 24);
    assert_eq!(t.app().graph().commits().len(), 3);
    t.press(".");
    assert_eq!(t.app().graph().commits().len(), 3);
    assert!(t.screen().contains("no folder to narrow to"), "{}", t.screen());
}

#[test]
fn author_filter_survives_the_folder_toggle() {
    let sb = two_folders();
    commit_as(&sb, ALICE, "app/alice.rs", "alice app");
    let mut t = TestDriver::at(Config::default(), &sb, &sb.path().join("app"), 100, 30);
    t.press("f").type_text("alice").press("enter");
    t.press(".");
    assert_eq!(summaries(&t), ["alice app"]);
    assert!(
        t.pane_title(PaneId::Graph)
            .contains("Graph [all · author Alice · folder app/]")
    );
    t.press(".");
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · author Alice]"));
}

#[test]
fn filter_panel_offers_the_start_folder() {
    let sb = two_folders();
    let mut t = TestDriver::at(Config::default(), &sb, &sb.path().join("app"), 100, 30);
    t.press("f").press("tab");
    t.wait_idle();
    assert!(t.screen().contains("current folder"), "{}", t.screen());
    t.press("enter");
    assert_eq!(summaries(&t), ["app two", "app one"]);
    assert_eq!(status_paths(&t), ["app/main.rs", "app/new.rs"], "Status follows");
}

#[test]
fn status_bulk_actions_stay_in_the_folder() {
    let sb = two_folders();
    let mut t = TestDriver::at(Config::default(), &sb, &sb.path().join("app"), 100, 30);
    t.press(".").press("2");

    // a stages only app/; lib/lib.rs was already staged and stays so.
    t.press("a").wait_idle();
    let st = t.app().status();
    assert!(st.all_entries().iter().all(|e| e.has_staged() && !e.has_unstaged()));
    t.press("a").wait_idle();
    let st = t.app().status();
    assert!(st.entries().iter().all(|e| !e.has_staged()), "unstaged app/");
    let lib = st.all_entries().iter().find(|e| e.path == "lib/lib.rs").unwrap();
    assert!(lib.has_staged(), "outside left staged");

    // D discards only app/.
    t.press("D");
    assert!(t.screen().contains("Discard all in app/"), "{}", t.screen());
    assert!(t.screen().contains("everything outside app/"));
    t.press("y").wait_idle();
    assert!(t.app().status().entries().is_empty());
    assert_eq!(t.app().status().all_entries().len(), 1);
    t.assert_pane_contains(PaneId::Status, "no changes in app/");
    assert_eq!(std::fs::read_to_string(sb.path().join("lib/lib.rs")).unwrap(), "2\n");
}

#[test]
fn status_stash_starts_on_the_folder() {
    let sb = two_folders();
    let mut t = TestDriver::at(Config::default(), &sb, &sb.path().join("app"), 100, 30);
    t.press(".").press("2").press("s");
    assert!(t.screen().contains("only app/"), "{}", t.screen());
    t.press("enter").wait_idle();
    assert!(t.app().status().entries().is_empty());
    let left: Vec<&str> = t.app().status().all_entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(left, ["lib/lib.rs"]);
}

/// Conventional commit subjects, one by Alice.
fn conventional() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "feat(graph): Add a thing");
    sb.commit_file("b.txt", "b\n", "fix: Mend it");
    commit_as(&sb, ALICE, "c.txt", "feat: Alice's feature");
    sb.commit_file("d.txt", "d\n", "docs: Describe it");
    sb
}

#[test]
fn slash_filters_commits_by_message() {
    let sb = conventional();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("/");
    insta::assert_snapshot!("message_filter_prompt", t.screen());

    // The presets are the repo's prefixes, most used first, then newest
    // first: ^feat, ^docs, ^fix. tab fills in the first; enter applies it.
    t.press("tab").press("enter");
    assert_eq!(summaries(&t), ["feat: Alice's feature", "feat(graph): Add a thing"]);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · message /^feat]"));
    insta::assert_snapshot!("message_filter_applied", t.screen());

    // / again starts from the pattern; tab steps on to the next preset.
    // The graph only shows feat commits now, but the presets are the same.
    t.press("/").press("tab").press("enter");
    assert_eq!(summaries(&t), ["docs: Describe it"]);

    // An author narrows it further, and the message stays.
    t.press("/").press("ctrl+u").type_text("^feat").press("enter");
    t.press("f").type_text("alice").press("enter");
    assert_eq!(summaries(&t), ["feat: Alice's feature"]);
    assert!(
        t.pane_title(PaneId::Graph)
            .contains("Graph [all · author Alice · message /^feat]")
    );

    // esc clears every part.
    t.press("esc");
    assert_eq!(t.app().graph().commits().len(), 4);
}

#[test]
fn message_filter_refuses_an_invalid_regex() {
    let sb = conventional();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("/").type_text("feat(").press("enter");
    assert!(matches!(t.app().modal(), Some(Modal::Prompt { .. })), "stays open");
    assert!(t.screen().contains("invalid regex: unclosed group"), "{}", t.screen());
    t.press("esc");
    assert!(!t.app().graph().is_filtered());

    // Typed with a capital, it matches case.
    t.press("/").type_text("Mend").press("enter");
    assert_eq!(summaries(&t), ["fix: Mend it"]);
    t.press("/").press("ctrl+u").type_text("mend").press("enter");
    assert_eq!(summaries(&t), ["fix: Mend it"]);
    t.press("/").press("ctrl+u").type_text("MEND").press("enter");
    assert!(t.app().graph().commits().is_empty());
    t.assert_pane_contains(PaneId::Graph, "no commits match the filter");
}

#[test]
fn message_presets_come_from_config() {
    let sb = conventional();
    let mut config = Config::default();
    config.graph.message_presets = Some(vec!["^fix".into()]);
    let mut t = TestDriver::with_size(config, &sb, 100, 30);
    t.press("/").press("tab").press("enter");
    assert_eq!(summaries(&t), ["fix: Mend it"]);
}

#[test]
fn no_presets_without_subject_prefixes() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("/");
    assert!(t.screen().contains("enter filter · esc cancel"), "{}", t.screen());
    t.press("tab");
    let Some(Modal::Prompt { input, .. }) = t.app().modal() else {
        panic!("prompt closed")
    };
    assert_eq!(input, "", "tab has nothing to fill in");
}
