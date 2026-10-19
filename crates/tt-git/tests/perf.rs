//! Opt-in performance checks against a real repository (read-only):
//!
//! ```sh
//! TTGIT_PERF_REPO=/path/to/big/repo cargo test --release -p tt-git --test perf -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use tt_git_core::{GitCli, LogScope, Repo};
use tt_git_graph::GraphBuilder;

fn perf_repo() -> Option<std::path::PathBuf> {
    std::env::var_os("TTGIT_PERF_REPO").map(Into::into)
}

#[test]
#[ignore = "needs TTGIT_PERF_REPO"]
fn load_and_layout() {
    let Some(path) = perf_repo() else { return };
    let repo = Repo::discover(&path, &GitCli::new(&path)).unwrap();

    let t = Instant::now();
    let refs = repo.refs().unwrap();
    let refs_time = t.elapsed();

    let t = Instant::now();
    let commits = repo.commits(&LogScope::All, 20_000).unwrap();
    let log_time = t.elapsed();

    let t = Instant::now();
    let mut ids = std::collections::HashMap::new();
    let mut intern = |oid: tt_git_core::Oid| {
        let n = ids.len() as u32;
        *ids.entry(oid).or_insert(n)
    };
    let mut g = GraphBuilder::new();
    let mut rows = Vec::with_capacity(commits.len());
    for c in &commits {
        let id = intern(c.oid);
        let parents: Vec<u32> = c.parents.iter().map(|p| intern(*p)).collect();
        rows.push(g.push(id, &parents));
    }
    let layout_time = t.elapsed();
    let max_lanes = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    // Rows wider than their inline capacity keep their cells on the heap.
    let spilled: Vec<_> = rows.iter().filter(|r| r.cells.spilled()).collect();
    let heap: usize = spilled
        .iter()
        .map(|r| r.cells.capacity() * size_of::<tt_git_graph::Cell>() + r.fillers.capacity() * size_of::<Option<u8>>())
        .sum();

    println!(
        "{} refs in {refs_time:?}; {} commits in {log_time:?}; layout {layout_time:?} (max {max_lanes} lanes)",
        refs.len(),
        commits.len()
    );
    // As the graph pane keeps them: packed, up to the 16 lanes it draws.
    let mut packed = tt_git_graph::Rows::new(16);
    for row in &rows {
        packed.push(row);
    }
    let kept: usize = packed.iter().map(|r| r.kept()).sum();
    println!(
        "rows: {} x {} bytes inline; {} spilled to the heap ({heap} bytes); packed: {kept} cells ({} bytes) + {} row heads; commits: {} x {} bytes",
        rows.len(),
        size_of::<tt_git_graph::Row>(),
        spilled.len(),
        kept * 2,
        packed.len(),
        commits.len(),
        size_of::<tt_git_core::Commit>()
    );
    assert!(layout_time < Duration::from_millis(500));
}

#[test]
#[ignore = "needs TTGIT_PERF_REPO"]
fn time_to_full_graph_in_the_real_binary() {
    let Some(path) = perf_repo() else { return };
    let total = {
        let repo = Repo::discover(&path, &GitCli::new(&path)).unwrap();
        repo.commits(&LogScope::All, 20_000).unwrap().len()
    };
    let start = Instant::now();
    let mut t = termlens::Terminal::builder()
        .size(160, 45)
        .timeout(Duration::from_secs(60))
        .current_dir(&path)
        .env("TTGIT_CONFIG_DIR", std::env::temp_dir().join("tt-git-perf-config"))
        .arg("--no-plugins")
        .spawn(env!("CARGO_BIN_EXE_tt-git"))
        .unwrap();
    t.wait_until(|s| s.contains("Graph [all] 1/")).unwrap();
    let first_paint = start.elapsed();
    let done = format!("1/{total}");
    t.wait_until(|s| s.contains(&done) && !s.contains("loading…")).unwrap();
    let full = start.elapsed();
    println!("first rows after {first_paint:?}; all {total} commits after {full:?}");
    if std::env::var_os("TTGIT_PERF_SHOW").is_some() {
        println!("{}", t.screen().text());
    }
    t.send_str("q").unwrap();
    t.wait_exit().unwrap();
}
