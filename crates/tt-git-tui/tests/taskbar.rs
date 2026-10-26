//! What the desktop app shows on its taskbar button: while git talks to a
//! remote (`App::take_job_progress`), busy while the job runs, an error for
//! two seconds after it fails, nothing for local writes; and with
//! `[ui] notifications` on, a badge for conflicts or a stopped merge or
//! rebase (`App::take_repo_alert`).

use tt_git_app::Config;
use tt_git_app::app::{JobProgress, RepoAlert};
use tt_git_core::InProgress;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Action;
use tt_git_tui::modal::GitOp;

/// `main` tracking `origin/main`, one commit ahead.
fn ahead_of_origin() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    sb.add_remote("origin");
    sb.git(&["push", "-q", "-u", "origin", "HEAD"]);
    sb.commit_file("a.txt", "2", "local work");
    sb
}

#[test]
fn a_push_is_busy_until_it_finishes() {
    let sb = ahead_of_origin();
    let mut t = TestDriver::new(&sb);
    assert_eq!(t.app_mut().take_job_progress(), None, "idle from the start");

    t.send_frame(Action::Run(GitOp::Push));
    assert_eq!(t.app_mut().take_job_progress(), Some(JobProgress::Busy));
    t.tick(4);
    assert_eq!(t.app_mut().take_job_progress(), None, "only handed out when it changes");

    t.wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(t.app_mut().take_job_progress(), Some(JobProgress::Idle));
}

#[test]
fn a_failed_fetch_shows_the_error_for_two_seconds() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    let missing = sb.path().join("no-such-remote");
    sb.git(&["remote", "add", "origin", &missing.to_string_lossy()]);
    let mut t = TestDriver::new(&sb);

    t.send_frame(Action::Run(GitOp::FetchAll));
    assert_eq!(t.app_mut().take_job_progress(), Some(JobProgress::Busy));
    t.wait_idle();
    assert!(t.app().error().is_some(), "the fetch failed");
    assert_eq!(t.app_mut().take_job_progress(), Some(JobProgress::Failed));

    t.tick(7);
    assert_eq!(t.app_mut().take_job_progress(), None, "still showing the error");
    t.tick(1);
    assert_eq!(t.app_mut().take_job_progress(), Some(JobProgress::Idle));
}

#[test]
fn local_writes_leave_the_taskbar_alone() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1", "base");
    sb.write("a.txt", "2");
    let mut t = TestDriver::new(&sb);

    t.send_frame(Action::Run(GitOp::Stage(vec!["a.txt".into()])));
    assert_eq!(t.app_mut().take_job_progress(), None);
    t.wait_idle();
    assert_eq!(t.app_mut().take_job_progress(), None);
}

/// `main` and `other` both change `f.txt`, so merging `other` stops on a
/// conflict.
fn conflicting() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file(
        "f.txt", "base
", "base",
    );
    sb.checkout_new("other");
    sb.commit_file(
        "f.txt", "theirs
", "theirs",
    );
    sb.checkout("main");
    sb.commit_file(
        "f.txt", "ours
", "ours",
    );
    sb
}

fn merge_other(t: &mut TestDriver) {
    t.send(Action::Run(GitOp::Merge { target: "other".into() }));
}

#[test]
fn a_conflicted_merge_is_an_alert_until_its_resolved() {
    let sb = conflicting();
    let mut config = Config::default();
    config.ui.notifications = true;
    let mut t = TestDriver::with_config(config, &sb);
    assert_eq!(t.app_mut().take_repo_alert(), None, "nothing to say at first");

    merge_other(&mut t);
    assert_eq!(t.app_mut().take_repo_alert(), Some(RepoAlert::Conflicts));
    assert_eq!(t.app_mut().take_repo_alert(), None, "only handed out when it changes");

    // Resolved and staged, the merge still waits to be committed.
    sb.write(
        "f.txt", "both
",
    );
    t.send(Action::Run(GitOp::Stage(vec!["f.txt".into()])));
    assert_eq!(
        t.app_mut().take_repo_alert(),
        Some(RepoAlert::Stopped(InProgress::Merge))
    );

    t.send(Action::Run(GitOp::Abort(InProgress::Merge)));
    assert_eq!(t.app_mut().take_repo_alert(), Some(RepoAlert::None));
}

#[test]
fn no_alert_with_notifications_off() {
    let sb = conflicting();
    let mut t = TestDriver::new(&sb);
    merge_other(&mut t);
    assert!(t.app().status().status().is_some_and(|s| s.in_progress.is_some()));
    assert_eq!(t.app_mut().take_repo_alert(), None);
}
