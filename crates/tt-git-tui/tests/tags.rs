use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::{BranchTab, PaneId};

fn tag_names(t: &TestDriver) -> Vec<String> {
    t.app()
        .branches()
        .visible_tags()
        .map(|r| r.short_name.clone())
        .collect()
}

#[test]
fn lightweight_tag_at_graph_commit() {
    let sb = Sandbox::new();
    let first = sb.commit("first");
    sb.commit("second");
    let mut t = TestDriver::new(&sb);
    t.press("j").press("T");
    assert!(t.screen().contains("New tag at"), "{}", t.screen());
    assert!(t.screen().contains("just a name for the commit"));
    t.type_text("v1").press("enter");
    assert!(t.app().modal().is_none());
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["cat-file", "-t", "v1"]), "commit");
    assert_eq!(sb.rev_parse("v1"), first);
    assert!(t.app().notice().unwrap_or_default().contains("create tag v1"));
    t.press("3").press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    assert_eq!(tag_names(&t), vec!["v1"]);
}

#[test]
fn annotated_tag_asks_for_a_message() {
    let sb = Sandbox::new();
    let head = sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("3").press("T");
    assert!(t.screen().contains("New tag at main"));
    t.type_text("v2").press("tab");
    assert!(t.screen().contains("asks for a message next"));
    t.press("enter");
    assert!(matches!(t.app().modal(), Some(Modal::Prompt { .. })));
    assert!(t.screen().contains("Message for tag v2"), "{}", t.screen());
    t.press("enter");
    assert!(t.app().modal().is_some(), "an empty message isn't submitted");
    t.type_text("Release two").press("enter");
    assert!(t.app().modal().is_none());
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["cat-file", "-t", "v2"]), "tag");
    assert_eq!(sb.rev_parse("v2^{commit}"), head);
    assert_eq!(
        sb.git(&["tag", "-l", "--format=%(contents:subject)", "v2"]),
        "Release two"
    );
}

#[test]
fn existing_tag_name_shows_git_error() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.git(&["tag", "v1"]);
    let mut t = TestDriver::new(&sb);
    t.press("T").type_text("v1").press("enter");
    let err = t.app().error().unwrap_or_default();
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn push_and_delete_a_tag_on_the_remote() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["tag", "v1"]);
    sb.git(&["tag", "v2"]);
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3").press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.press("/").type_text("v1").press("enter");
    assert_eq!(tag_names(&t), vec!["v1"]);

    t.press("P");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git_in(&remote, &["tag", "-l"]), "v1", "only the selected tag");

    t.press("d");
    let screen = t.screen();
    assert!(screen.contains("Delete local tag v1?"), "{screen}");
    assert!(screen.contains("r here and on origin"), "{screen}");
    t.press("r");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git_in(&remote, &["tag", "-l"]), "");
    assert_eq!(sb.git(&["tag", "-l"]), "v2");
}

#[test]
fn deleting_a_tag_only_here_keeps_the_remote_one() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["tag", "v1"]);
    sb.git(&["push", "-q", "origin", "v1"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("]").press("d").press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["tag", "-l"]), "");
    assert_eq!(sb.git_in(&remote, &["tag", "-l"]), "v1");
}

#[test]
fn without_a_remote_delete_is_yes_or_no() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.git(&["tag", "v1"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("]").press("d");
    assert!(!t.screen().contains("here and on"), "{}", t.screen());
    t.press("r");
    assert!(t.app().modal().is_some(), "r does nothing without a remote");
    t.press("y");
    assert_eq!(sb.git(&["tag", "-l"]), "");
}

#[test]
fn p_still_pushes_the_branch_outside_the_tags_tab() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.commit("more");
    let mut t = TestDriver::new(&sb);
    t.press("3").press("P");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git_in(&remote, &["rev-parse", "main"]), sb.rev_parse("HEAD"));
    assert_eq!(t.app().focus(), PaneId::Branches);
}
