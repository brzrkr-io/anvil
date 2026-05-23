//! Criterion benchmarks for the VT parser hot path.
//!
//! Run:  cargo bench -p anvil-term --bench parser
//!
//! Each bench feeds 64 KiB of representative data through the parser and
//! reports throughput in bytes/sec so results are comparable across machines.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use anvil_term::Parser;
use anvil_term::parser::Handler;

// ── Null handler — discards every event; no allocation ───────────────────────

struct Null;

impl Handler for Null {
    fn print(&mut self, _: char) {}
    fn execute(&mut self, _: u8) {}
    fn csi_dispatch(&mut self, _: &[u8], _: &[u16], _: u8) {}
    fn esc_dispatch(&mut self, _: &[u8], _: u8) {}
    fn osc_dispatch(&mut self, _: &[u8]) {}
}

// ── Input generators ─────────────────────────────────────────────────────────

const BENCH_SIZE: usize = 64 * 1024; // 64 KiB

/// Repeated "hello world\n" — pure printable ASCII.
fn plain_ascii_input() -> Vec<u8> {
    let unit = b"hello world\n";
    let mut v = Vec::with_capacity(BENCH_SIZE);
    while v.len() < BENCH_SIZE {
        let remaining = BENCH_SIZE - v.len();
        let chunk = &unit[..unit.len().min(remaining)];
        v.extend_from_slice(chunk);
    }
    v
}

/// Repeated "\x1b[31mred\x1b[0m" — colour-escape-heavy CSI traffic.
fn csi_heavy_input() -> Vec<u8> {
    let unit = b"\x1b[31mred\x1b[0m";
    let mut v = Vec::with_capacity(BENCH_SIZE);
    while v.len() < BENCH_SIZE {
        let remaining = BENCH_SIZE - v.len();
        let chunk = &unit[..unit.len().min(remaining)];
        v.extend_from_slice(chunk);
    }
    v
}

/// Mixed BMP + astral codepoints.
///
/// Each unit: "café𝄞" — 'é' is U+00E9 (2 bytes), '𝄞' is U+1D11E (4 bytes).
fn unicode_input() -> Vec<u8> {
    // "café𝄞\n" encoded as UTF-8 bytes.
    let unit = "café𝄞\n".as_bytes();
    let mut v = Vec::with_capacity(BENCH_SIZE + unit.len());
    while v.len() < BENCH_SIZE {
        v.extend_from_slice(unit);
    }
    v.truncate(BENCH_SIZE);
    v
}

// ── Benches ──────────────────────────────────────────────────────────────────

fn bench_parser_plain_ascii(c: &mut Criterion) {
    let input = plain_ascii_input();
    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("plain_ascii", input.len()),
        &input,
        |b, data| {
            b.iter(|| {
                let mut p = Parser::new();
                let mut h = Null;
                p.feed(&mut h, data);
            });
        },
    );
    group.finish();
}

fn bench_parser_csi_heavy(c: &mut Criterion) {
    let input = csi_heavy_input();
    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("csi_heavy", input.len()),
        &input,
        |b, data| {
            b.iter(|| {
                let mut p = Parser::new();
                let mut h = Null;
                p.feed(&mut h, data);
            });
        },
    );
    group.finish();
}

fn bench_parser_unicode(c: &mut Criterion) {
    let input = unicode_input();
    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("unicode", input.len()),
        &input,
        |b, data| {
            b.iter(|| {
                let mut p = Parser::new();
                let mut h = Null;
                p.feed(&mut h, data);
            });
        },
    );
    group.finish();
}

criterion_group!(
    benches,
    bench_parser_plain_ascii,
    bench_parser_csi_heavy,
    bench_parser_unicode
);
criterion_main!(benches);
