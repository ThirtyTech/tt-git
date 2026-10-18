//! Layout throughput on a synthetic history shaped like a busy repo:
//! a long mainline with frequent short-lived feature branches merged back.

use criterion::{Criterion, criterion_group, criterion_main};
use tt_git_graph::{GraphBuilder, NodeId};

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

fn bench(c: &mut Criterion) {
    let dag = synthetic(100_000);
    c.bench_function("layout 100k commits", |b| {
        b.iter(|| {
            let mut g = GraphBuilder::new();
            let mut cells = 0usize;
            for (id, parents) in &dag {
                cells += g.push(*id, parents).cells.len();
            }
            cells
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
