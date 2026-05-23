//! Criterion benchmarks for `AtlasPainter::glyph_slot`.
//!
//! Run:  cargo bench -p anvil-platform --bench glyph_cache
//!
//! macOS-only (requires a Metal device and CoreText).
//!
//! Two scenarios:
//!   cold  — cache miss: each call uses a distinct codepoint so every call
//!           goes through rasterization and atlas upload.
//!   hot   — cache hit: same codepoint every call; only the HashMap lookup
//!           is exercised after the first call.

#![cfg(target_os = "macos")]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use anvil_platform::AtlasPainter;
use anvil_platform::font::Font;
use anvil_render::{FontMetrics, GlyphRasterizer};

const PIXEL_SIZE: f64 = 28.0; // 14pt @ 2× retina

fn metrics() -> FontMetrics {
    FontMetrics {
        cell_w: 14.0,
        cell_h: PIXEL_SIZE,
        descent: 6.0,
    }
}

/// Build an AtlasPainter with "Menlo" (always present on macOS).
/// Falls back to "Monaco" and then panics if neither is available.
fn make_atlas() -> AtlasPainter {
    let font = Font::init_first_available(&["Menlo", "Monaco", "Courier New"], PIXEL_SIZE)
        .expect("no monospace font found");
    AtlasPainter::new_with_default_device(font)
        .expect("no Metal device — run on real macOS hardware")
        .expect("AtlasPainter construction failed")
}

fn bench_glyph_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("glyph_cache");

    // ── Cold: cache miss — iterate over distinct printable ASCII codepoints ──
    group.bench_function(BenchmarkId::new("glyph_slot", "cold_miss"), |b| {
        // Printable ASCII: 0x21..=0x7E (94 glyphs). Cycling through them
        // means each b.iter() gets the same slot sequence but starts fresh
        // every iter because we rebuild the atlas each time.
        let codepoints: Vec<u32> = (0x21u32..=0x7E).collect();
        let m = metrics();
        b.iter(|| {
            let mut atlas = make_atlas();
            for &cp in &codepoints {
                let _ = atlas.glyph_slot(cp, m);
            }
        });
    });

    // ── Hot: cache hit — same codepoint 1 000 times after initial warmup ────
    group.bench_function(BenchmarkId::new("glyph_slot", "hot_hit"), |b| {
        let m = metrics();
        let mut atlas = make_atlas();
        // Prime the cache so the first cold miss is amortised before timing.
        let _ = atlas.glyph_slot(b'A' as u32, m);

        b.iter(|| {
            // 'A' is always in cache from the warmup above.
            for _ in 0..1_000u32 {
                let _ = atlas.glyph_slot(b'A' as u32, m);
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_glyph_cache);
criterion_main!(benches);
