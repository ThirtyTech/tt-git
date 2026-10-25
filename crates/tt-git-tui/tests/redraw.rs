//! What makes the real runner draw a new frame, and graph loads that a
//! newer load replaced.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tt_git_core::time::Clock;
use tt_git_core::{Commit, GitCli, LogFilter, Repo};
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::draw::{Draw, Hits};
use tt_git_tui::jobs::Ctx;
use tt_git_tui::panes::{GraphPane, GraphScope};
use tt_git_tui::tui_theme::Theme;
use tt_git_tui::{Action, loaders};

#[test]
fn only_input_and_actions_that_change_things_redraw() {
    let sb = Sandbox::new();
    for i in 0..3 {
        sb.commit(&format!("commit {i}"));
    }
    let mut t = TestDriver::new(&sb);
    t.app_mut().take_redraw();

    let release = KeyEvent {
        kind: KeyEventKind::Release,
        ..KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
    };
    t.event(Event::Key(release));
    assert!(!t.app_mut().take_redraw(), "key release");

    for kind in [
        MouseEventKind::Moved,
        MouseEventKind::Up(crossterm::event::MouseButton::Left),
    ] {
        t.event(Event::Mouse(MouseEvent {
            kind,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(!t.app_mut().take_redraw(), "{kind:?}");
    }

    // No plugins and a fixed clock: nothing a tick could change.
    t.send(Action::Tick);
    assert!(!t.app_mut().take_redraw(), "tick");

    t.event(Event::Resize(80, 20));
    assert!(t.app_mut().take_redraw(), "resize");

    t.press("j");
    assert!(t.app_mut().take_redraw(), "key press");
    assert!(!t.app_mut().take_redraw(), "the flag clears");

    t.send(Action::Notify("hello".into()));
    assert!(t.app_mut().take_redraw(), "action");
}

#[derive(Debug)]
struct StepClock(AtomicI64);

impl Clock for StepClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn commit(oid: &str, time: i64) -> Commit {
    Commit {
        oid: oid.parse().unwrap(),
        parents: Default::default(),
        author_name: "Ann".into(),
        author_email: "ann@example.com".into(),
        author_time: time,
        commit_time: time,
        summary: "a commit".into(),
    }
}

#[test]
fn graph_dates_go_stale_when_their_text_would_change() {
    const THEN: i64 = 1_700_000_000;
    let clock = Arc::new(StepClock(AtomicI64::new(THEN + 30)));
    let mut graph = GraphPane::new(clock.clone());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = Ctx::new(tx);
    let load = graph.begin_load();
    graph.apply_batch(
        load.generation(),
        vec![commit("aaaaaaaaaa", THEN), commit("bbbbbbbbbb", THEN - 3_000)],
        true,
        &ctx,
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 6)).unwrap();
    let mut draw = |graph: &mut GraphPane| {
        terminal
            .draw(|f| graph.draw(f, f.area(), true, &Theme::default(), &mut Hits::default()))
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let screen = draw(&mut graph);
    assert!(screen.contains(" now "), "{screen}");
    assert!(screen.contains(" 50m "), "{screen}");

    // "now" lasts until the first commit is a minute old; "50m" until 51.
    assert!(!graph.dates_stale(THEN + 30));
    assert!(!graph.dates_stale(THEN + 59));
    assert!(graph.dates_stale(THEN + 60));
    assert!(graph.dates_stale(THEN + 29), "the clock went back");

    clock.0.store(THEN + 60, Ordering::SeqCst);
    let screen = draw(&mut graph);
    assert!(screen.contains(" 1m "), "{screen}");
    assert!(screen.contains(" 51m "), "{screen}");
    // Next: 2m and 52m.
    assert!(!graph.dates_stale(THEN + 60));
    assert!(!graph.dates_stale(THEN + 119));
    assert!(graph.dates_stale(THEN + 120));
}

#[test]
fn superseded_graph_load_stops_streaming() {
    let sb = Sandbox::new();
    for i in 0..5 {
        sb.commit(&format!("commit {i}"));
    }
    let git = GitCli::new(sb.path()).with_env(sb.env());
    let repo = Repo::discover(sb.path(), &git).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = Ctx::new(tx);
    let mut graph = GraphPane::new(Arc::new(StepClock(AtomicI64::new(0))));

    let old = graph.begin_load();
    let current = graph.begin_load();
    assert!(!old.is_current());
    assert!(current.is_current());

    let load = |token| {
        loaders::load_graph(
            repo.clone(),
            GraphScope::All,
            None,
            LogFilter::default(),
            token,
            1000,
            None,
            &ctx,
        )
    };
    match load(old.clone()) {
        Action::GraphBatch {
            generation,
            commits,
            done,
        } => {
            assert_eq!(generation, old.generation());
            assert!(commits.is_empty(), "stopped at the first commit");
            assert!(done);
        }
        other => panic!("unexpected {other:?}"),
    }
    match load(current.clone()) {
        Action::GraphBatch { commits, done, .. } => {
            assert_eq!(commits.len(), 5);
            assert!(done);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Both ran `git log`; the stopped one isn't logged as a failure.
    let logs: Vec<_> = repo
        .git()
        .log()
        .entries()
        .into_iter()
        .filter(|c| c.args.starts_with("log "))
        .collect();
    assert_eq!(logs.len(), 2, "{logs:?}");
    assert!(logs.iter().all(|c| c.success == Some(true)), "{logs:?}");
}
