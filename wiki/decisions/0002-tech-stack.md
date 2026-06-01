---
status: active
type: decision
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../../context/2026-05-21-m1-complete.md
confidence: high
---

# 0002 — Tech Stack: Zig + Metal + AppKit

## Status

Partially superseded. The Rust port (0004) was archived on tag `rust-port-archive`
(2026-05-29). The `zig` branch returned to and extends the Zig implementation
described here. The webview-hybrid model (M3) was superseded by [[0005-render-host]]
(native Metal only). The core Zig + Metal + AppKit stack is current.
The content below documents the Zig architecture that remains active.

## Context

Anvil needed a native macOS terminal that avoids Electron, avoids
Swift/Objective-C boilerplate, and is fully controllable from a CLI dev
workflow (no Xcode IDE requirement). The team already uses Zig for related
work, so language consistency was a factor.

Key constraint: Xcode Command Line Tools only — no full Xcode installation.
This ruled out any approach that requires offline Metal shader compilation
(`xcrun metal` / Xcode build phases), since that tool is not included in CLT.

## Decision

- **Language: Zig 0.16** — the sole application language. Build system:
  `build.zig` / `build.zig.zon`; dependency: `zig-objc` for Objective-C
  message sends.
- **Window / app lifecycle: AppKit via zig-objc** — `NSApplication`,
  `NSWindow`, `NSView`, `NSTimer`. No Swift or .xib files.
- **GPU rendering: Metal via CAMetalLayer** — the terminal grid is rasterized
  CPU-side (CoreText → BGRA8 bitmap) and uploaded as a `MTLTexture` once per
  frame. A single full-screen quad shader composites it.
- **Runtime shader compilation: `newLibraryWithSource:`** — MSL source is
  embedded as a Zig string literal in `src/render/metal.zig` and compiled by
  the Metal driver at app startup. This eliminates any offline `metal` toolchain
  requirement and means Xcode Command Line Tools alone suffices.
- **Text: CoreText + CoreGraphics** — glyph metrics and rasterization via
  hand-written extern declarations (`src/render/capi.zig`); no `@cImport`.

## Consequences

- `zig build run` is the only command needed; no Xcode project, no `.xcconfig`.
- Shader changes require an app restart (compiled once at init), not a rebuild.
- CPU rasterization means no glyph atlas GPU acceleration yet; this is tracked
  as tech debt in `todo.txt` and does not block M2.
- `@cImport` is avoided for Apple APIs; stability across Zig releases requires
  maintaining `capi.zig` by hand. See [[concepts/zig-0.16-gotchas]] for the
  rationale and other 0.16 API changes.
- The architecture is detailed in [[concepts/console-architecture]].
