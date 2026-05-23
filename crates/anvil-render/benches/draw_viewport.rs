//! Criterion benchmarks for the per-frame viewport draw loop.
//!
//! Run:  cargo bench -p anvil-render --bench draw_viewport
//!
//! Two scenarios:
//!   full_redraw   — DirtySet::all (every row redrawn)
//!   damaged_row   — DirtySet::none with a single dirty row (damage-tracking win)
//!
//! Reports µs/frame for each scenario; the ratio gives the damage-tracking
//! speedup factor.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use anvil_render::{
    CursorConfig, CursorParams, CursorStyle, FoldedBlocks, FontMetrics, GlyphPainter, PixelRect,
    Raster, draw_viewport,
};
use anvil_term::{DirtySet, Terminal};
use anvil_theme::MINERAL_DARK;
use anvil_workspace::selection::Selection;

// ── No-op glyph painter ───────────────────────────────────────────────────────

/// Stub painter: writes nothing, allocates nothing. The benchmark measures the
/// draw-loop dispatch overhead, not glyph rasterization.
struct NullPainter;

impl GlyphPainter for NullPainter {
    fn draw_glyph(
        &mut self,
        _codepoint: u32,
        _dest: PixelRect,
        _fg: [u8; 3],
        _metrics: FontMetrics,
        _pixels: &mut [u8],
        _bitmap_width: usize,
        _bitmap_height: usize,
    ) {
    }
}

// ── Bench helpers ─────────────────────────────────────────────────────────────

const COLS: usize = 80;
const ROWS: usize = 24;

/// Build a terminal with 80×24 cells filled with mixed text and colour escapes.
fn make_terminal() -> Terminal {
    let mut t = Terminal::new(COLS, ROWS, 1000);
    // Fill every row: alternate plain text and a colour-escape sequence so we
    // exercise both plain-cell and SGR-colour paths in draw_cell.
    let line_plain = "hello world from Anvil terminal benchmark suite  ok\r\n";
    let line_color = "\x1b[31mred text\x1b[0m \x1b[32mgreen\x1b[0m \x1b[34mblue\x1b[0m end\r\n";
    for row in 0..ROWS {
        if row % 2 == 0 {
            t.feed(line_plain.as_bytes());
        } else {
            t.feed(line_color.as_bytes());
        }
    }
    // Drain initial dirty state so subsequent dirty sets are clean.
    let _ = t.take_dirty_rows();
    t
}

/// Shared metrics and raster used by both scenarios.
fn make_raster_and_metrics() -> (Raster, FontMetrics) {
    // 2× retina pixels: cell 14×28 device px, descent 6 px.
    let metrics = FontMetrics {
        cell_w: 14.0,
        cell_h: 28.0,
        descent: 6.0,
    };
    // Raster sized for (tab_bar=1 row) + 24 terminal rows, 80 cols wide.
    let px_w = (COLS as f64 * metrics.cell_w) as usize;
    let px_h = ((ROWS + 1) as f64 * metrics.cell_h) as usize;
    (Raster::new(px_w, px_h), metrics)
}

fn bench_draw_viewport(c: &mut Criterion) {
    let mut group = c.benchmark_group("draw_viewport");

    // ── Full redraw ───────────────────────────────────────────────────────────
    group.bench_function(BenchmarkId::new("scenario", "full_redraw"), |b| {
        let mut terminal = make_terminal();
        let (mut raster, metrics) = make_raster_and_metrics();
        let mut painter = NullPainter;
        let theme = MINERAL_DARK;
        let selection = Selection::default();

        b.iter(|| {
            let dirty = DirtySet::all(ROWS);
            draw_viewport(
                &mut raster,
                &mut painter,
                &mut terminal,
                metrics,
                &theme,
                0.0, // scroll_pos
                0.0, // overscroll
                selection,
                None, // search
                1,    // top_bar_rows (tab bar)
                Some(CursorParams {
                    ax: 0.0,
                    ay: 0.0,
                    blink_phase: 0.0,
                    cfg: CursorConfig {
                        style: CursorStyle::Block,
                        blink: false,
                    },
                }),
                0.0, // rule_x_start
                0.0, // rule_x_end
                FoldedBlocks::empty(),
                Some(&dirty),
            );
        });
    });

    // ── Single dirty row (damage-tracking) ───────────────────────────────────
    group.bench_function(BenchmarkId::new("scenario", "damaged_row_12"), |b| {
        let mut terminal = make_terminal();
        let (mut raster, metrics) = make_raster_and_metrics();
        let mut painter = NullPainter;
        let theme = MINERAL_DARK;
        let selection = Selection::default();

        b.iter(|| {
            let mut dirty = DirtySet::none(ROWS);
            dirty.mark(12);
            draw_viewport(
                &mut raster,
                &mut painter,
                &mut terminal,
                metrics,
                &theme,
                0.0,
                0.0,
                selection,
                None,
                1,
                Some(CursorParams {
                    ax: 0.0,
                    ay: 12.0,
                    blink_phase: 0.0,
                    cfg: CursorConfig {
                        style: CursorStyle::Block,
                        blink: false,
                    },
                }),
                0.0,
                0.0,
                FoldedBlocks::empty(),
                Some(&dirty),
            );
        });
    });

    group.finish();
}

criterion_group!(benches, bench_draw_viewport);
criterion_main!(benches);
