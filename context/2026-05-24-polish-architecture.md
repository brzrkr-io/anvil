---
date: 2026-05-24
kind: architecture
goal: Polish batch — chrome font, ligatures, SGR variants, diff tint, palette strip, kubectl worker
---

# Polish Batch Architecture (A/B/C/D)

## A. Chrome font (#6) + ligatures (#7) + SGR variants (#9)

Shared: `anvil-platform/src/font.rs`, `CoreTextPainter`, mask cache.

**Faces.** Introduce `FontFace { Regular, Bold, Italic, BoldItalic, Chrome }` and `Font::init_face(name, pixel_size, face, ligatures)`. App-shell owns `FontBundle { grid: [Font; 4], chrome: Font }` and passes it where today it passes `&Font`. Authoritative `FontMetrics` come from `grid[Regular]`; bold/italic of a monospace family must share advance — assert at init. Chrome loads at 11pt × scale with its own metrics.

**Painters.** One painter per face; `CoreTextPainter` internals unchanged but each face owns its mask cache (masks differ). `PainterBundle { grid: [CoreTextPainter; 4], chrome: CoreTextPainter }`. In `draw.rs`, the grid sink selects a painter from `cell.attrs & (BOLD|ITALIC)`. Chrome callers (`tabbar.rs`, `statusbar.rs`, `searchbar.rs`) take `chrome_painter + chrome_metrics` explicitly — they already use `glyph_at(x_px, y_px)`, no row math changes.

**Atlas choice.** Four small per-face mask caches, not one keyed by `(codepoint, face)`. Lifetime + cell-size invalidation is already wired per painter; cross-face sharing buys nothing because masks differ. Memory bounded — chrome is a tiny glyph set; italic/bold rarely cover the BMP.

**Ligatures.** Enable `kCTFontFeatureContextualAlternates` via `CTFontDescriptor` feature settings at face creation, gated by `ligatures: bool` (on for grid, off for chrome). BlexMonoNerdFontMono inherits Plex Mono `calt`. CRITICAL: the mask cache keys on glyph index; true ligature rendering needs a per-run shaping pass over consecutive same-style cells. This commit only enables the feature flag — visible ligatures land when shaping arrives in a follow-up.

**Touches:** `anvil-platform/src/font.rs`, `anvil-platform/src/lib.rs`, `anvil/src/main.rs`, `anvil-render/src/draw.rs`, `anvil-render/src/{tabbar,statusbar,searchbar}.rs`.
**Risks:** italic face absent in Blex build → synthesize via affine skew, log. Feature flag without shaping → no visible change yet; scaffolding only. Bundle threading touches many call sites — single Wave-5 builder.
**Open Qs:** none blocking.

## B. Inline diff colorization (#13)

**Detector on `anvil-term`.** Add `Block::diff_kind: DiffKind { None, Unified }`. Run once in `terminal.rs::block_from_mark` when 133;D finalizes the block: scan up to 200 output rows, classify Unified iff the first two non-blank lines start `--- ` and `+++ `.

**Render gate.** `draw.rs::draw_cell` already has a per-frame block lookup (fff27e3). When `block.diff_kind == Unified` and the row is in `[output_line, end_line)`, peek the first cell: `+` → row bg `theme.verified`; `-` → `theme.failure`; `@` → muted accent. Use `Raster::fill_pixel_rect` — no per-cell loops.

**Touches:** `anvil-term/src/terminal.rs` (field + detector), `anvil-render/src/draw.rs` (row tint), `anvil-theme/src/lib.rs` (tints if absent).
**Risks:** false positive on `--- end ---` → require `+++` on next non-blank line. Alt-screen has no marks, skips naturally. Cost = one cell read per tinted row.

## C. Command palette as a 28pt strip (#14)

**Hierarchy.** Keep `Webview` as an `AnvilView` subview. Add `palette_strip_pt: f64` (0 hidden, 28 shown, animated). Window splits: chrome row → palette strip → terminal. When `palette_strip_pt > 0`, terminal cell rows are computed against `(content_h - chrome_h - palette_strip_pt) * scale`. Webview frame is `(0, content_h - chrome_h - palette_strip_pt, content_w, palette_strip_pt)` in points.

**IPC.** Existing `anvil-control` Inbound/Outbound suffices. Add Outbound `PaletteResized { height_px }` so JS switches between modal and slim layouts.

**Animation.** Anim state on `App` (matches `tick()` at main.rs:2049): `palette_anim: { from_pt, to_pt, t0, dur_ms }`. `tick()` advances `palette_strip_pt`, sets `app.dirty = true` only while in flight. Eased cubic over 100ms. During flight, redraw the terminal at the smaller height **without** resizing the PTY (clip + letterbox at the top); fire one PTY resize on settle. Avoids per-frame `ioctl(TIOCSWINSZ)`.

**Touches:** `anvil-platform/src/webview.rs` (rect-aware frame helper), `anvil/src/main.rs` (anim + layout + single resize on settle), `anvil-control/src/lib.rs` (Outbound), `ui/palette/`.
**Risks:** mid-anim clip vs. PTY mismatch — bounded ~100ms, masked by the strip. Focus handoff: set first-responder on settle, not per tick.

## D. kubectl context worker (#20)

**Worker.** Mirror `recent_files` worker (main.rs:3160): `thread::spawn`, `mpsc::sync_channel(1)` for cwd hints, 5s cadence, immediate re-poll on cwd change. Out channel sends `KubeResult { cluster, namespace, env_kind }`. Invoke `kubectl config view --minify -o jsonpath=…` with a 1s timeout; any failure → unavailable. Suppress sends when unchanged.

**Cache.** `pub struct KubeCtx { cluster: String, namespace: String, env_kind: EnvKind }`, `EnvKind { Prod, Staging, Dev }`. Classifier on cluster name: `prod-*` | `*-prod` → Prod; `staging-*` | `stg-*` → Staging; else Dev. Place in `anvil-prompt-core` (already owns prompt env-kind logic) and re-export.

**App wiring.** `LocalContext` gains `kube: Option<KubeCtx>`. `App` adds `kube_tx`, `kube_rx`. `tick()` drains `kube_rx` alongside `git_rx` (~main.rs:1180); cwd-change site (main.rs:1177) does `kube_tx.try_send(cwd)`.

**Status bar.** `statusbar.rs` reads `local_ctx.kube`; when `Some`, renders `⎈ <cluster>/<ns>` in the chrome font (item A). Tint: Prod → `attention`, Staging → `trace`, Dev → muted. When `None`, segment is omitted.

**Touches:** new `anvil/src/kube.rs`, `anvil/src/main.rs`, `anvil-render/src/statusbar.rs`, `anvil-prompt-core/src/lib.rs`.
**Risks:** `kubectl` shells out — 1s timeout, log on error, never block. Heuristic classifier — unknown → Dev. Suppress NoContext spam on equality.

---

## Builder commits (ordered)

1. **A1** — `FontFace` + `FontBundle` scaffold; assert grid faces share advance. No call-site changes.
2. **A2** — thread `chrome_painter + chrome_metrics` through `tabbar.rs`, `statusbar.rs`, `searchbar.rs`.
3. **A3** — SGR face selection in `draw_cell`; affine-skew fallback for missing italic.
4. **A4** — enable `calt` on grid descriptor; document shaping follow-up.
5. **B** — `Block::diff_kind` + detector + row-fill tint in `draw.rs`.
6. **C1** — palette strip layout + Webview rect; single PTY resize on settle. No animation.
7. **C2** — `palette_anim` easing + Outbound `PaletteResized` + slim JS layout.
8. **D** — `kube.rs` worker + `LocalContext.kube` + status segment with env tint.
