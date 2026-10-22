//! "Update tt-git" in the palette. Nothing here goes online: the release
//! check's result is sent in directly, and the install is never confirmed.

use tt_git_core::update::CURRENT_VERSION;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Action;
use tt_git_tui::modal::{GitOp, Modal};

#[test]
fn the_palette_offers_update_and_refuses_a_binary_it_cant_replace() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("update tt");
    let screen = t.screen();
    assert!(screen.contains("Update tt-git"), "{screen}");

    // The test binary isn't named tt-git, so it stops before going online.
    t.press("enter").wait_idle();
    let error = t.app().error().unwrap_or_default().to_string();
    assert!(error.starts_with("can't update tt-git:"), "{error}");
    assert!(error.contains("isn't named tt-git"), "{error}");
}

#[test]
fn a_newer_release_asks_before_installing() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.send(Action::UpdateChecked(Ok("v99.0.0".into())));
    let Some(Modal::Confirm { title, message, op, .. }) = t.app().modal() else {
        panic!("no confirm dialog: {}", t.screen());
    };
    assert_eq!(title, "Update tt-git");
    assert_eq!(
        *message,
        format!(
            "tt-git v99.0.0 is out (you have {CURRENT_VERSION}).\nDownload and install it now?\n\n\
             Restart tt-git afterwards to use it."
        )
    );
    assert_eq!(*op, GitOp::SelfUpdate { tag: "v99.0.0".into() });
    t.press("n").wait_idle();
    assert!(t.app().modal().is_none());
}

#[test]
fn the_same_or_older_release_is_up_to_date() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.send(Action::UpdateChecked(Ok(format!("v{CURRENT_VERSION}"))));
    assert!(t.app().modal().is_none());
    assert_eq!(
        t.app().notice(),
        Some(format!("tt-git {CURRENT_VERSION} is up to date").as_str())
    );

    t.send(Action::UpdateChecked(Err("curl: (6) Could not resolve host".into())));
    assert_eq!(
        t.app().error(),
        Some("checking for a tt-git update failed:\ncurl: (6) Could not resolve host")
    );
}

#[test]
fn a_finished_update_says_to_restart() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    let op = GitOp::SelfUpdate { tag: "v99.0.0".into() };
    t.send(Action::OpDone {
        op: op.clone(),
        result: Ok("Installed tt-git v99.0.0".into()),
        head: None,
    });
    assert_eq!(
        t.app().notice(),
        Some("✓ installed tt-git v99.0.0; restart tt-git to use it")
    );

    t.send(Action::OpDone {
        op,
        result: Err("the install script failed (exit 1)\n\ntt-git install: checksum mismatch".into()),
        head: None,
    });
    let error = t.app().error().unwrap_or_default();
    assert!(error.starts_with("update tt-git to v99.0.0 failed:"), "{error}");
    assert!(error.contains("checksum mismatch"), "{error}");
}
