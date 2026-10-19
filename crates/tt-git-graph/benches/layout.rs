//! Layout throughput on synthetic histories: one shaped like a busy repo
//! (a long mainline with frequent short-lived feature branches merged
//! back), and a wide one with dozens of long-lived lines of history open at
//! once, as in big repos with many active branches. Also how fast rows
//! pack into [`Rows`], as the graph pane keeps them.

use criterion::{Criterion, criterion_group, criterion_main};
use tt_git_graph::{GraphBuilder, NodeId, Rows};

/// Newest-first topo-ordered DAG of about `n` commits.
fn synthetic(n: usize) -> Vec<(NodeId, Vec<NodeId>)> {
    let mut out = Vec::with_capacity(n);
    let mut seed: u64 = 42;
    let mut rand = move |m: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 33) % m
    };
    for i in 0..n {
        let i = i as u64;
        let remaining = n as u64 - i - 1;
        let parents = match remaining {
            0 => vec![],
            _ if rand(5) == 0 && remaining > 3 => {
                vec![(i + 1) as NodeId, (i + 2 + rand(remaining.min(40) - 1)) as NodeId]
            }
            _ => vec![(i + 1 + rand(remaining.min(3))) as NodeId],
        };
        out.push((i as NodeId, parents));
    }
    out
}

/// Newest-first topo-ordered DAG of `n` commits on `lines` long-lived
/// lines of history taking turns, now and then merging another line in.
fn wide(n: usize, lines: usize) -> Vec<(NodeId, Vec<NodeId>)> {
    let mut seed: u64 = 7;
    let mut rand = move |m: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 33) % m
    };
    (0..n)
        .map(|i| {
            let mut parents = Vec::new();
            if i + lines < n {
                parents.push((i + lines) as NodeId);
            }
            let other = i + 1 + rand(lines as u64 * 3) as usize;
            if rand(10) == 0 && other < n && other != i + lines {
                parents.push(other as NodeId);
            }
            (i as NodeId, parents)
        })
        .collect()
}

fn bench_layout(c: &mut Criterion, name: &str, dag: &[(NodeId, Vec<NodeId>)]) {
    let mut max = 0;
    let mut g = GraphBuilder::new();
    for (id, parents) in dag {
        max = max.max(g.push(*id, parents).width());
    }
    println!("{name}: up to {max} lanes");
    c.bench_function(&format!("layout {name}"), |b| {
        b.iter(|| {
            let mut g = GraphBuilder::new();
            let mut cells = 0usize;
            for (id, parents) in dag {
                cells += g.push(*id, parents).cells.len();
            }
            cells
        })
    });
    c.bench_function(&format!("layout and pack {name}"), |b| {
        b.iter(|| {
            let mut g = GraphBuilder::new();
            let mut rows = Rows::new(16);
            for (id, parents) in dag {
                rows.push(&g.push(*id, parents));
            }
            rows.len()
        })
    });
}

fn bench(c: &mut Criterion) {
    bench_layout(c, "100k commits", &synthetic(100_000));
    bench_layout(c, "20k commits, 40 lines", &wide(20_000, 40));
}

criterion_group!(benches, bench);
criterion_main!(benches);
