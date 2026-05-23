//! Criterion benchmarks for `Grid::resize`.
//!
//! Run:  cargo bench -p anvil-term --bench grid_resize
//!
//! Covers the same size matrix exercised by the regression test suite so
//! we can track resize cost across the cases that actually appear in use.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use anvil_term::Grid;

struct ResizeCase {
    name: &'static str,
    w1: usize,
    h1: usize,
    w2: usize,
    h2: usize,
}

const CASES: &[ResizeCase] = &[
    ResizeCase {
        name: "grow_both",
        w1: 4,
        h1: 3,
        w2: 8,
        h2: 6,
    },
    ResizeCase {
        name: "shrink_both",
        w1: 8,
        h1: 6,
        w2: 4,
        h2: 3,
    },
    ResizeCase {
        name: "grow_cols_only",
        w1: 4,
        h1: 3,
        w2: 8,
        h2: 3,
    },
    ResizeCase {
        name: "shrink_rows_only",
        w1: 4,
        h1: 6,
        w2: 4,
        h2: 3,
    },
    ResizeCase {
        name: "degenerate_1x1",
        w1: 8,
        h1: 4,
        w2: 1,
        h2: 1,
    },
    ResizeCase {
        name: "no_op",
        w1: 4,
        h1: 3,
        w2: 4,
        h2: 3,
    },
    ResizeCase {
        name: "typical_80x24_grow",
        w1: 80,
        h1: 24,
        w2: 220,
        h2: 50,
    },
    ResizeCase {
        name: "typical_220x50_shrink",
        w1: 220,
        h1: 50,
        w2: 80,
        h2: 24,
    },
];

fn bench_grid_resize(c: &mut Criterion) {
    let mut group = c.benchmark_group("grid_resize");
    for case in CASES {
        group.bench_with_input(BenchmarkId::new("resize", case.name), case, |b, c| {
            b.iter(|| {
                let mut g = Grid::new(c.w1, c.h1);
                // Print a few characters so there is something to copy.
                for ch in "hello world".chars() {
                    g.print(ch);
                }
                g.resize(c.w2, c.h2);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_grid_resize);
criterion_main!(benches);
