# Anvil Performance Benchmarks

This document describes the benchmark suite, explains which hot paths each
bench targets, and tracks baseline numbers across perf pushes.

---

## Running the criterion micro-benchmarks

Each crate with a `benches/` directory has benchmarks registered under
`[[bench]]` entries in its `Cargo.toml`. All use criterion 0.5.

```sh
# Parser throughput (plain ASCII, CSI-heavy, unicode) — reports bytes/sec
cargo bench -p anvil-term --bench parser

# Single-scenario quick run (criterion warm-up + one measurement round)
cargo bench -p anvil-term --bench parser -- --quick

# Grid resize cost across the standard matrix
cargo bench -p anvil-term --bench grid_resize

# Viewport draw loop: full-redraw vs single-row damage
cargo bench -p anvil-render --bench draw_viewport

# Glyph atlas cache: cold miss vs hot hit (macOS only, requires Metal)
cargo bench -p anvil-platform --bench glyph_cache
```

Criterion writes HTML reports to `target/criterion/`.

---

## Running the head-to-head vtebench comparison

```sh
./scripts/bench-vs-alacritty.sh
```

Prerequisites (script will tell you if they are missing):

| Tool        | Install                      |
|-------------|------------------------------|
| `vtebench`  | `cargo install vtebench`     |
| `alacritty` | `brew install alacritty`     |

The script builds Anvil in release mode automatically.

### vtebench workloads

| Workload            | What it exercises                            |
|---------------------|----------------------------------------------|
| `dense_cells`       | Rapid full-screen cell writes                |
| `light_cells`       | Sparse updates (most cells unchanged)        |
| `scrolling`         | Continuous scroll (insert-line traffic)      |
| `scrolling_in_region` | Scroll within a DECSTBM region            |
| `unicode`           | High Unicode codepoint density               |

### Headless limitation

vtebench drives terminals by wrapping them as a PTY child process. Anvil is a
GUI app and cannot be wrapped this way yet. The script times Alacritty
automatically and prints the manual steps for Anvil.

**Manual Anvil procedure:**

1. Launch Anvil.
2. In an Anvil terminal tab run (example for `dense_cells`):

   ```sh
   vtebench dense_cells -c 80 -r 24 -b 10485760 | pv -ab > /dev/null
   ```

3. Record elapsed time and MB/s. Convert to ms:
   `ms = (bytes / (MB_s * 1e6)) * 1000`

4. Fill in the Current Numbers table below.

---

## Hot paths

### `anvil-term::parser::Parser::feed`

The entry point for all terminal output. Every byte written to the PTY passes
through this function. It is a byte-at-a-time DFA (Paul Williams VT500-series)
with inline UTF-8 decoding. Benchmarked in `crates/anvil-term/benches/parser.rs`.

Optimization targets: reduce dispatch overhead per byte; consider lookup-table
transitions for the ground state (which handles >90% of bytes in typical output).

### `anvil-render::draw::draw_viewport`

Called once per display frame (60 Hz). Iterates all dirty rows, resolves colors,
dispatches glyph painting. The damage-tracking (`DirtySet`) path skips unchanged
rows so most frames touch only 1–3 rows instead of all 24.

Benchmarked in `crates/anvil-render/benches/draw_viewport.rs`. The
`full_redraw` / `damaged_row_12` ratio quantifies the damage-tracking speedup.

### `anvil-platform::glyph_atlas::AtlasPainter::glyph_slot`

Resolves a codepoint to an atlas slot. A cache hit is a single HashMap lookup.
A cache miss involves CoreText rasterization, BGRA→R8 conversion, shelf-packer
allocation, and a Metal `replaceRegion` upload. Benchmarked in
`crates/anvil-platform/benches/glyph_cache.rs`.

---

## Current Numbers

*Populate after the first bench run on the target machine.*

### Criterion parser throughput (64 KiB input)

| Scenario     | Throughput   | Date | Machine |
|--------------|--------------|------|---------|
| plain_ascii  | —            | —    | —       |
| csi_heavy    | —            | —    | —       |
| unicode      | —            | —    | —       |

### Criterion draw_viewport (µs/frame, 80×24)

| Scenario       | µs/frame | Date | Machine |
|----------------|----------|------|---------|
| full_redraw    | —        | —    | —       |
| damaged_row_12 | —        | —    | —       |
| ratio          | —        | —    | —       |

### vtebench vs Alacritty (10 MiB per workload, ms)

| Workload              | Anvil | Alacritty | Delta |
|-----------------------|-------|-----------|-------|
| dense_cells           | —     | —         | —     |
| light_cells           | —     | —         | —     |
| scrolling             | —     | —         | —     |
| scrolling_in_region   | —     | —         | —     |
| unicode               | —     | —         | —     |
