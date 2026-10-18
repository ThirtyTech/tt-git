use tt_git_core::branches::unify;
use tt_git_core::{BranchState, CheckoutPlan, GitCli, Repo};
use tt_git_testkit::Sandbox;

fn open(sb: &Sandbox) -> Repo {
    Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover")
}

fn entries(repo: &Repo) -> Vec<tt_git_core::BranchEntry> {
    let refs = repo.refs().unwrap();
    let head = repo.head().unwrap();
    let checkouts = repo.checkouts(500).unwrap();
    unify(&refs, &head, &checkouts, |r| repo.ref_activity(r))
}

/// A repo whose remote has a branch someone else pushed.
fn with_remote_only_branch() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["switch", "-q", "-c", "their-feature"]);
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "their work"]);
    sb.git_in(&other, &["push", "-q", "-u", "origin", "their-feature"]);
    sb.git(&["fetch", "-q", "origin"]);
    sb
}

#[test]
fn checkout_remote_only_branch_creates_tracking_branch() {
    let sb = with_remote_only_branch();
    let repo = open(&sb);
    let list = entries(&repo);
    let theirs = list.iter().find(|e| e.name == "their-feature").expect("listed");
    assert!(theirs.local.is_none());
    assert!(matches!(theirs.state(), BranchState::RemoteOnly { .. }));

    let plan = theirs.checkout_plan().unwrap();
    assert_eq!(
        plan,
        CheckoutPlan::Track {
            name: "their-feature".into(),
            remote_ref: "origin/their-feature".into()
        }
    );
    repo.checkout(&plan).expect("checkout succeeds without prompting");

    assert_eq!(sb.current_branch(), "their-feature");
    assert_eq!(
        sb.git(&["rev-parse", "--abbrev-ref", "their-feature@{upstream}"]),
        "origin/their-feature"
    );

    // Now it is a single in-sync entry at the top (most recent checkout).
    let list = entries(&repo);
    let first = &list[0];
    assert_eq!(first.name, "their-feature");
    assert!(first.is_head);
    assert_eq!(first.state(), BranchState::InSync);
    assert_eq!(list.iter().filter(|e| e.name == "their-feature").count(), 1);
}

#[test]
fn checkout_with_conflicting_changes_reports_git_error() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "main\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "other\n", "change f");
    sb.checkout("main");
    sb.write("f.txt", "local edit\n");

    let repo = open(&sb);
    let err = repo.checkout(&CheckoutPlan::Local("other".into())).unwrap_err();
    let stderr = err.stderr().unwrap_or_default();
    assert!(stderr.contains("would be overwritten"), "{stderr}");
    assert_eq!(sb.current_branch(), "main");
}

#[test]
fn recently_checked_out_branch_sorts_first() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("older");
    sb.commit("older work");
    sb.checkout("main");
    sb.checkout_new("newer");
    sb.commit("newer work");
    sb.checkout("main");
    // Checking out "older" again makes it the most recently active.
    sb.tick();
    let repo = open(&sb);
    repo.checkout(&CheckoutPlan::Local("older".into())).unwrap();
    let list = entries(&repo);
    assert_eq!(list[0].name, "older");
}

#[test]
fn create_rename_delete() {
    let sb = Sandbox::new();
    let c = sb.commit("base");
    let repo = open(&sb);
    repo.create_branch("topic", &c).unwrap();
    assert_eq!(sb.current_branch(), "topic");
    repo.rename_branch("topic", "topic2").unwrap();
    assert_eq!(sb.current_branch(), "topic2");
    sb.checkout("main");
    repo.delete_branch("topic2", false).unwrap();
    assert!(!sb.git(&["branch", "--list"]).contains("topic2"));
}

#[test]
fn unmerged_delete_needs_force() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("wip");
    sb.commit("unmerged work");
    sb.checkout("main");
    let repo = open(&sb);
    let err = repo.delete_branch("wip", false).unwrap_err();
    assert!(err.stderr().unwrap_or_default().contains("not fully merged"), "{err}");
    repo.delete_branch("wip", true).unwrap();
}

#[test]
fn fetch_and_delete_remote_branch() {
    let sb = with_remote_only_branch();
    let repo = open(&sb);
    repo.delete_remote_branch("origin", "their-feature").unwrap();
    repo.fetch_all().unwrap();
    assert!(entries(&repo).iter().all(|e| e.name != "their-feature"));
}
