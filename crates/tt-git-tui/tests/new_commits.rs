//! `App::take_new_commits`: what a fetch brought in, and what was committed
//! from outside the window, for the desktop app's notifications
//! (`[ui] notifications`, `[ui.alerts]`).

use tt_git_app::config::Alert;
use tt_git_app::modal::GitOp;
use tt_git_app::new_commits::BranchCommits;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::{Action, Config};

/// `main` tracking `origin/main`, and another clone of the remote that
/// pushes `n` commits to it.
fn behind_origin(n: usize) -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    let url = sb.add_remote("origin");
    sb.git(&["branch", "-q", "-u", "origin/main"]);
    let other = sb.clone_to(&url, "other");
    for i in 0..n {
        let name = format!("theirs{i}.txt");
        std::fs::write(other.join(&name), "x").unwrap();
        sb.git_in(&other, &["add", "."]);
        sb.git_in(&other, &["commit", "-q", "-m", &name]);
    }
    sb.git_in(&other, &["push", "-q", "origin", "main"]);
    sb
}

fn notifying() -> Config {
    let mut config = Config::default();
    config.ui.notifications = true;
    config
}

#[test]
fn a_fetch_brings_new_commits_and_the_first_load_none() {
    let sb = behind_origin(3);
    let mut t = TestDriver::with_config(notifying(), &sb);
    t.wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None, "nothing on the first load");

    sb.git(&["fetch", "-q", "origin"]);
    t.send(Action::Refresh).wait_idle();
    let new = t.app_mut().take_new_commits().expect("new commits");
    assert_eq!(
        new.branches,
        vec![BranchCommits {
            branch: "origin/main".into(),
            count: 3,
            head_upstream: true,
            local: false,
            worktree: None,
        }]
    );
    assert_eq!(new.summary(), "3 new commits on origin/main");
    assert_eq!(t.app_mut().take_new_commits(), None, "taken");

    // A refresh with nothing new brings none.
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None);
}

#[test]
fn nothing_while_notifications_are_off() {
    let sb = behind_origin(1);
    let mut t = TestDriver::new(&sb);
    t.wait_idle();
    sb.git(&["fetch", "-q", "origin"]);
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None);
}

/// A repository with `feature` checked out in the worktree `feature-wt`
/// next to it, and the worktree's path.
fn with_worktree() -> (Sandbox, std::path::PathBuf) {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    sb.git(&["branch", "-q", "feature"]);
    let wt = sb.root().join("feature-wt");
    sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "feature"]);
    (sb, wt)
}

/// Commits a new file in `dir`, as an agent working there would.
fn commit_in(sb: &Sandbox, dir: &std::path::Path, name: &str) {
    std::fs::write(dir.join(name), name).unwrap();
    sb.git_in(dir, &["add", "."]);
    sb.git_in(dir, &["commit", "-q", "-m", name]);
}

#[test]
fn a_commit_in_another_worktree_is_new_and_names_it() {
    let (sb, wt) = with_worktree();
    let mut t = TestDriver::with_config(notifying(), &sb);
    commit_in(&sb, &wt, "agent1.txt");
    commit_in(&sb, &wt, "agent2.txt");
    t.send(Action::Refresh).wait_idle();
    let new = t.app_mut().take_new_commits().expect("new commits");
    assert_eq!(
        new.branches,
        vec![BranchCommits {
            branch: "feature".into(),
            count: 2,
            head_upstream: false,
            local: true,
            worktree: Some("feature-wt".into()),
        }]
    );
    assert_eq!(new.summary(), "2 new commits on feature (worktree feature-wt)");

    // The next one within the minute waits for it, then comes on its own.
    commit_in(&sb, &wt, "agent3.txt");
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None, "held");
    t.tick(60 * 4);
    assert_eq!(t.app_mut().take_new_commits().map(|n| n.total()), Some(1));
}

#[test]
fn local_commits_can_be_turned_off() {
    let (sb, wt) = with_worktree();
    let mut config = notifying();
    config.ui.alerts.set(Alert::LocalCommits, false);
    let mut t = TestDriver::with_config(config, &sb);
    commit_in(&sb, &wt, "agent.txt");
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None);
}

#[test]
fn remote_commits_can_be_turned_off() {
    let sb = behind_origin(2);
    let mut config = notifying();
    config.ui.alerts.set(Alert::RemoteCommits, false);
    let mut t = TestDriver::with_config(config, &sb);
    sb.git(&["fetch", "-q", "origin"]);
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None);
}

#[test]
fn a_commit_from_this_window_isnt_news_but_one_from_outside_is() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    let mut t = TestDriver::with_config(notifying(), &sb);

    sb.write("b.txt", "2");
    sb.git(&["add", "."]);
    t.send(Action::Run(GitOp::Commit {
        message: Some("mine".into()),
        amend: false,
    }))
    .wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    t.send(Action::Refresh).wait_idle();
    assert_eq!(t.app_mut().take_new_commits(), None, "its own commit");

    // From a terminal in the same worktree, it's news.
    commit_in(&sb, sb.path(), "c.txt");
    t.send(Action::Refresh).wait_idle();
    let new = t.app_mut().take_new_commits().expect("new commits");
    assert_eq!(new.summary(), "1 new commit on main");
}
