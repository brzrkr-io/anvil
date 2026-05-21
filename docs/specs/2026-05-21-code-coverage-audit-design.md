# Code Coverage Audit & Test Backfill — Design

Date: 2026-05-21
Status: Approved (design); pending spec review

## Goal

Give Caldera Console a measurable test-coverage baseline and backfill tests,
module by module, until every testable module reaches **90% line coverage**.
"Testable" excludes the Objective-C / Metal glue, which is out of scope for the
90% target.

## Current State

- `src/` is 4715 lines across 15 `.zig` files.
- 119 `test` blocks exist across 13 of 15 files. Two files have none:
  - `src/render/capi.zig` (80 lines) — C-API glue / `extern` functions.
  - `src/render/metal.zig` (190 lines) — Metal GPU rendering.
- No coverage tooling exists. `build.zig` defines only `run` and `test` steps;
  the `test` step runs `b.addTest` on the exe module rooted at `main.zig`.
- Test discovery is already sound: `src/main.zig:447` has a
  `test { _ = @import(...) }` aggregator importing 8 modules, and the
  `terminal/` sub-files are pulled in transitively via `terminal.zig`. The
  119 tests are expected to all run today — Phase 3 verifies the count.

## Approach

Approach A: tooling-first, module-by-module, coverage-driven. Add kcov and a
`zig build coverage` step, capture a baseline, then backfill tests one module
at a time, re-running coverage after each module so "done" is a measured
number rather than a judgment call.

Rejected alternatives:
- **CI coverage gate** — the repo has no CI at all and Metal needs a macOS
  runner; that is its own project. Possible follow-up, out of scope here.
- **Literal 100%, including `capi.zig`/`metal.zig`** — would require mock
  harnesses for the Obj-C/Metal boundary; high effort, brittle, low value.

## Scope

**Testable modules (subject to the 90% line-coverage target):**

- `terminal/terminal.zig`, `terminal/parser.zig`, `terminal/grid.zig`,
  `terminal/scrollback.zig`, `terminal/cell.zig`
- `config/config.zig`, `config/theme.zig`
- `app/keys.zig`
- `render/raster.zig`, `render/color.zig`, `render/font.zig`

**Tested but not kcov-measured:**

- `pty/pty.zig` — kcov livelocks tracing the child processes the pty tests
  spawn on macOS (confirmed during Phase 1). It is excluded from the kcov
  coverage root (`src/coverage_root.zig`); its tests still run under
  `zig build test`, and its coverage is assessed by reading rather than by a
  kcov number.

**Excluded from the 90% target (best-effort only):**

- `render/capi.zig`, `render/metal.zig` — Obj-C / GPU glue.
- `main.zig` — app bootstrap; covered incidentally, no target.

## Phases

1. **kcov spike.** `brew install kcov`, then confirm kcov produces a sane
   per-file line report when run over a Zig test binary on this Apple Silicon
   Mac. Verify: a readable report with non-zero coverage for a known-tested
   file (e.g. `terminal/parser.zig`).
2. **`zig build coverage` step.** Add a build step that runs kcov over the
   test artifact with `--include-pattern=src/`, emitting an HTML + summary
   report into a build output directory. Verify: one command produces the
   report; no manual kcov invocation needed.
3. **Baseline.** Run `zig build coverage`; record per-file line coverage and
   the total `test` count. Verify: numbers captured in the wiki.
4. **Backfill.** Module by module in this order — `terminal/` → `config/` →
   `pty/` → `app/` → `render/`. For each module: read the coverage report,
   add tests for uncovered lines and branches, use `std.testing.allocator`
   for anything that allocates, re-run coverage. Verify: each testable module
   reaches ≥90% line coverage and `zig build test` stays green.
5. **Closeout.** Add a `wiki/` page documenting the coverage workflow and
   final numbers; append `wiki/log.md` per AGENTS.md wiki rules.

## kcov / build integration

The `coverage` step wraps the test artifact produced by `b.addTest`:

- Reuse the existing `exe_tests` compile step (or a dedicated one).
- `b.addSystemCommand(&.{"kcov", "--clean", "--include-pattern=src/"})`,
  add an output directory arg, then `addArtifactArg(exe_tests)` so the build
  graph depends on the compiled test binary and passes its path to kcov.
- kcov reads DWARF debug info; Zig Debug-mode test binaries carry it, so no
  extra build flags are needed.

## Risks

- **kcov on Apple Silicon can be flaky.** Mitigated by the Phase 1 spike — if
  kcov cannot produce a usable report, stop and report before building the
  `coverage` step. Fallback options would be reconsidered at that point.
- **Coverage granularity.** kcov reports line coverage, not full branch
  coverage. The 90% target is therefore line coverage; branch gaps are caught
  by reading the code during each module's backfill, not enforced by a number.
- **Build-cache interaction.** kcov output is handled via a build output
  directory arg so Zig's caching does not serve a stale report.

## Verification (definition of done)

- `zig build coverage` produces a per-file line-coverage report.
- `zig build test` passes.
- Every testable module listed above is at ≥90% line coverage.
- `wiki/` has a coverage-workflow page and `wiki/log.md` is appended.

## Out of Scope

- CI / a coverage gate on push.
- 100% coverage of `render/capi.zig` and `render/metal.zig`.
- Enforced branch-coverage thresholds.
