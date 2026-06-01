---
status: active
type: operation
created: 2026-05-21
updated: 2026-05-29
sources: []
confidence: high
---

# Code Coverage

## Running tests

`.zig/zig build test` runs all unit tests. This is the primary feedback loop.

## Coverage target

Testable modules are held at **≥90% line coverage**. The Metal renderer and
AppKit platform layers (`src/platform/`) have no headless tests for GPU/ObjC
surfaces; coverage there is judged by reading and manual QA.

## Headless render check

`./zig-out/bin/anvil --dump /tmp/x.png` exercises the full Metal pipeline
without a GUI session. Use this to catch runtime shader errors that unit tests
cannot reach.

## Historical note

Before the Zig rewrite (active `zig` branch), coverage ran via `cargo test
--workspace` against the archived Rust port (`rust-port-archive` tag).
