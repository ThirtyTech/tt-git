//! `App::take_new_commits`: what a fetch brought in, for the desktop app's
//! notifications (`[ui] notifications`).

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
