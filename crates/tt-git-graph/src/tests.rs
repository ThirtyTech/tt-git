use super::*;

/// Lays out commits written as `("name", &["parent", ...])`, newest first,
/// and renders one line per commit: graph, then name.
fn draw(commits: &[(&str, &[&str])]) -> String {
    let mut ids: Vec<String> = Vec::new();
    let mut id = |name: &str| -> NodeId {
        if let Some(i) = ids.iter().position(|n| n == name) {
            return i as NodeId;
        }
        ids.push(name.to_string());
        (ids.len() - 1) as NodeId
    };
    let interned: Vec<(NodeId, Vec<NodeId>)> = commits
        .iter()
        .map(|(name, parents)| (id(name), parents.iter().map(|p| id(p)).collect()))
        .collect();
    let rows = layout(interned.iter().map(|(i, p)| (*i, p.as_slice())));
    let width = rows.iter().map(|r| r.to_ascii().chars().count()).max().unwrap_or(0);
    rows.iter()
        .zip(commits)
        .map(|(row, (name, _))| {
            let g = row.to_ascii();
            let pad = width - g.chars().count();
            format!("{g}{}  {name}", " ".repeat(pad))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn linear() {
    insta::assert_snapshot!(draw(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]), @r"
    ●  c
    ●  b
    ●  a
    ");
}

#[test]
fn feature_branch_merged() {
    // main: a - b - m(merge of f2)
    // feature:   \- f1 - f2 /
    insta::assert_snapshot!(draw(&[
        ("m", &["b", "f2"]),
        ("f2", &["f1"]),
        ("b", &["a"]),
        ("f1", &["a"]),
        ("a", &[]),
    ]), @r"
    ◉─╮  m
    │ ●  f2
    ● │  b
    │ ●  f1
    ●─╯  a
    ");
}

#[test]
fn two_branch_tips_share_a_parent() {
    insta::assert_snapshot!(draw(&[
        ("main", &["base"]),
        ("topic", &["base"]),
        ("base", &["root"]),
        ("root", &[]),
    ]), @r"
    ●    main
    │ ●  topic
    ●─╯  base
    ●    root
    ");
}

#[test]
fn octopus_merge() {
    insta::assert_snapshot!(draw(&[
        ("m", &["a", "b", "c"]),
        ("c", &["base"]),
        ("b", &["base"]),
        ("a", &["base"]),
        ("base", &[]),
    ]), @r"
    ◉─┬─╮  m
    │ │ ●  c
    │ ● │  b
    ● │ │  a
    ●─┴─╯  base
    ");
}

#[test]
fn merge_into_existing_lane_joins_it() {
    // x merges p, which y's lane (column 0) is already waiting for.
    insta::assert_snapshot!(draw(&[
        ("y", &["p"]),
        ("x", &["a", "p"]),
        ("a", &["p"]),
        ("p", &[]),
    ]), @r"
    ●    y
    ├─◉  x
    │ ●  a
    ●─╯  p
    ");
}

#[test]
fn crossing_lines() {
    // a (column 0) merges `far`, which opens column 2 across y's line in column 1.
    insta::assert_snapshot!(draw(&[
        ("x", &["a"]),
        ("y", &["base"]),
        ("a", &["b", "far"]),
        ("far", &["b"]),
        ("b", &["base"]),
        ("base", &[]),
    ]), @r"
    ●      x
    │ ●    y
    ◉─┼─╮  a
    │ │ ●  far
    ●─┼─╯  b
    ●─╯    base
    ");
}

#[test]
fn freed_lane_is_reused_by_later_tip() {
    insta::assert_snapshot!(draw(&[
        ("t1", &["a"]),
        ("t2", &["a"]),
        ("a", &["b"]),
        ("t3", &["b"]),
        ("b", &[]),
    ]), @r"
    ●    t1
    │ ●  t2
    ●─╯  a
    │ ●  t3
    ●─╯  b
    ");
}

#[test]
fn disconnected_roots() {
    insta::assert_snapshot!(draw(&[
        ("a2", &["a1"]),
        ("b2", &["b1"]),
        ("a1", &[]),
        ("b1", &[]),
    ]), @r"
    ●    a2
    │ ●  b2
    ● │  a1
      ●  b1
    ");
}

#[test]
fn parents_beyond_the_loaded_page_stay_open() {
    let mut b = GraphBuilder::new();
    b.push(0, &[1]);
    b.push(2, &[3]);
    let row = b.push(1, &[9]);
    assert_eq!(row.to_ascii(), "● │");
    assert_eq!(b.open_lanes(), 2, "lanes waiting for unloaded commits stay open");
}

#[test]
fn criss_cross_merge() {
    insta::assert_snapshot!(draw(&[
        ("m1", &["a", "b"]),
        ("m2", &["b", "a"]),
        ("a", &["base"]),
        ("b", &["base"]),
        ("base", &[]),
    ]), @r"
    ◉─╮    m1
    ├─┼─◉  m2
    ● │ │  a
    │ ●─╯  b
    ●─╯    base
    ");
}

/// Small deterministic PRNG so the invariant test needs no dependencies.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A random DAG in a valid topological order (newest first): commit `i`'s
/// parents all have larger indices.
fn random_dag(rng: &mut Lcg, n: usize) -> Vec<(NodeId, Vec<NodeId>)> {
    (0..n)
        .map(|i| {
            let remaining = (n - i - 1) as u64;
            let parents = if remaining == 0 || rng.below(20) == 0 {
                vec![]
            } else {
                let count = match rng.below(10) {
                    0 => 2,
                    1 if remaining > 2 => 3,
                    _ => 1,
                };
                (0..count)
                    .map(|_| (i as u64 + 1 + rng.below(remaining.min(12))) as NodeId)
                    .collect()
            };
            (i as NodeId, parents)
        })
        .collect()
}

#[test]
fn invariants_hold_on_random_dags() {
    for seed in 0..300 {
        let mut rng = Lcg(seed);
        let n = 2 + rng.below(60) as usize;
        let dag = random_dag(&mut rng, n);
        let mut builder = GraphBuilder::new();
        let rows: Vec<Row> = dag.iter().map(|(id, p)| builder.push(*id, p)).collect();

        for (k, row) in rows.iter().enumerate() {
            assert!(row.cells[row.node_lane].node, "seed {seed} row {k}: node cell");
            assert_eq!(row.cells.iter().filter(|c| c.node).count(), 1, "seed {seed} row {k}");
            assert_eq!(row.cells.len(), row.fillers.len());

            // Horizontal lines are continuous.
            for i in 0..row.cells.len() {
                let right = row.cells[i].bits & bits::RIGHT != 0;
                let next_left = row.cells.get(i + 1).is_some_and(|c| c.bits & bits::LEFT != 0);
                assert_eq!(right, next_left, "seed {seed} row {k} col {i}: horizontal continuity");
                assert_eq!(right, row.fillers[i].is_some(), "seed {seed} row {k} col {i}: filler");
            }

            // Vertical lines are continuous between consecutive rows.
            if let Some(next) = rows.get(k + 1) {
                let cols = row.cells.len().max(next.cells.len());
                for i in 0..cols {
                    let down = row.cells.get(i).is_some_and(|c| c.bits & bits::DOWN != 0);
                    let up = next.cells.get(i).is_some_and(|c| c.bits & bits::UP != 0);
                    assert_eq!(down, up, "seed {seed} rows {k}->{} col {i}: vertical continuity", k + 1);
                }
            }

            // A node connects downward iff the commit has parents.
            let has_parents = !dag[k].1.is_empty();
            let node_down = row.cells[row.node_lane].bits & bits::DOWN != 0;
            assert_eq!(node_down, has_parents, "seed {seed} row {k}: node down");
        }
        assert_eq!(builder.open_lanes(), 0, "seed {seed}: every lane reached its commit");
    }
}

#[test]
fn glyph_table_covers_all_bit_patterns() {
    for b in 0..16u8 {
        let cell = Cell {
            bits: b,
            node: false,
            color: 0,
        };
        let g = cell.glyph('●');
        assert!(b == 0 || g != ' ', "bits {b:04b} should draw something");
    }
    assert_eq!(
        Cell {
            bits: 0,
            node: true,
            color: 0
        }
        .glyph('◉'),
        '◉'
    );
}
