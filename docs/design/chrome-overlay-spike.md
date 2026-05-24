# Chrome Overlay Spike — Architecture

Move Anvil's app chrome from Metal rasters into a transparent, full-window
WKWebView layered above the existing `CAMetalLayer`. Terminal grid stays in
Metal. Spike scope: prove the layered model with the bottom status bar only.
Tab bar, block headers, palette consolidation, agent panel come later.

## Assumptions

- The existing palette WKWebView (`crates/anvil-platform/src/webview.rs`)
  already does the load-bearing pieces: `drawsBackground = NO` via KVC, CSS
  `background: transparent`, script-message handler wired through
  `AppHandler::webview_message`. We reuse this pattern.
- That webview is currently a subview of the Metal-backed terminal view
  (`container.addSubview(wv_as_view)` at `webview.rs:183`). For an always-on
  overlay we need a different container (see below).
- The current palette is modal. The new chrome webview is **persistent and
  always visible**; the palette stays modal and continues to layer on top.
- `pointer-events: none` on the WebView body with `auto` on chrome regions
  reliably forwards clicks to the underlying `CAMetalLayer`. Standard
  Warp/Zed pattern; confirmed on macOS WebKit.

## View Hierarchy

Today `NSWindow.contentView` is `AnvilTerminalView` (Metal-backed). Refactor:

```
NSWindow.contentView = AnvilContainerView (plain layer-backed NSView)
  ├─ AnvilTerminalView   (Metal-backed, first responder, fills bounds)
  └─ AnvilChromeWebView  (WKWebView, transparent, fills bounds, on top)
        └─ (later) palette WKWebView, on top of chrome when visible
```

AppKit z-orders subviews by insertion order. Both children autoresize to fill.
`FullSizeContentView` is unchanged. Container has `wantsLayer = YES`,
`layer.isOpaque = YES`; Metal layer paints the window background.

## Hit Testing — chosen rule

**Full-window transparent WebView with CSS-region pointer-events.** Body
default `pointer-events: none`; chrome strips opt in with `auto`. Transparent
regions fall through to `AnvilTerminalView` unchanged.

Rejected alternative: a smaller WebView covering only chrome rects.
Pixel-perfect alignment of two siblings under live resize is exactly what
we're trying to escape, and future popovers/toasts would need re-layering.

## Keyboard Focus — chosen rule

Terminal stays first responder. Chrome WebView returns `false` for
`acceptsFirstResponder`. Clicking a chrome element fires
`postMessage` and Rust handles it without moving focus. The palette
(modal) keeps its current `makeFirstResponder` behavior on show/hide.

For the spike's status bar there are no focusable elements; this collapses
to "terminal always has focus." Tab bar in phase 2 follows the same rule.

## Frame Timing & Resize

`windowDidResize:` sequence:

1. `AppHandler::resize(w, h, in_live)` fires (existing path).
2. Rust resizes the Metal grid and **synchronously** repaints (existing).
3. Rust pushes one `Outbound::ChromeState` so the WebView can react if it
   needs to (it mostly doesn't — CSS handles it).
4. The WebView's own compositor catches up on its next tick.

We accept the WebView lagging the Metal layer by ≤1 frame during a fast
drag. The Metal black-screen bug under resize is orthogonal: the overlay
doesn't cause it and doesn't block fixing it later.

## IPC Contract

Add to `crates/anvil-control/src/bridge.rs`, reusing the existing enums:

**Outbound:** `ChromeState { tabs: Vec<TabInfo>, status: StatusFields,
dims: Dims }` where `StatusFields { cwd, exit, agent, clock }`. Phase 1
only renders `status`; `tabs` is sent but unused, just to lock the shape.

**Inbound:** `Chrome(ChromeAction)` with variants `SelectTab { id }`,
`CloseTab { id }`, `NewTab`, `ReorderTab { from, to }`, `OpenPalette`.
Phase 1 only emits `OpenPalette` (so a status-bar click can confirm
the round-trip end-to-end).

Disambiguate from the palette's existing `Ready / Invoke / Dismiss`
via the `type` discriminator (e.g. `"chrome.select_tab"` vs `"invoke"`).

## State Sync Rule

**Push on change, coalesce per tick.** `App` holds chrome state and a
`chrome_dirty: bool`. Any mutation (cwd via shell-integration, exit
code, clock minute rollover, tab rename) sets the bit. The existing
60 Hz `tick()` flushes one `Outbound::ChromeState` per tick when dirty,
then clears it.

Per-change push spams `evaluateJavaScript:`; a separate debounce timer
adds latency and complexity. The tick already runs; piggyback. Worst-
case latency ~16 ms, below the perceptible threshold for chrome.

## What Stays in Metal (phase 1)

Stays: terminal cells, cursor, selection, block accent stripes, tab bar
(deferred to phase 2 — too much hit-test surface for a first slice),
dividers. Moves to HTML: bottom status bar only. Deferred (phases 3+):
block headers, agent panel, cheatsheet, search bar, palette
consolidation onto this WebView.

## Failure Modes

- **HTML fails to load.** Detection: no `Inbound::Ready` within 500 ms of
  `Webview::init`. Degraded mode: log, fall back to the existing Metal
  status bar behind a `chrome_overlay_active` flag.
- **IPC drops a message.** Not possible: WKWebView script messages are
  in-process. No queue.
- **WebView compositor blocks during resize.** ≤1-frame lag; accepted.
- **`evaluateJavaScript:` errors after reload.** We don't reload; if we
  did, the `Ready` handshake catches it.

## New / Modified Files

New:

- `ui/chrome/index.html` — self-contained HTML/CSS/JS, same shape as
  `ui/palette/index.html`. Renders the bottom status strip.

Modified:

- `crates/anvil-platform/src/webview.rs` — add `persistent: bool` to
  `WebviewConfig`. `false` = current modal palette behavior; `true` =
  visible on init, never moves first responder, `acceptsFirstResponder`
  returns false.
- `crates/anvil-platform/src/appkit.rs` — introduce `AnvilContainerView`
  (plain layer-backed NSView, autoresizes subviews) as the window's
  content view. The terminal view becomes its child, not the content
  view itself.
- `crates/anvil-control/src/bridge.rs` — add `Outbound::ChromeState`
  and `Inbound::Chrome(ChromeAction)` with `"type"` discriminators and
  unit tests in the existing `tests` module.
- `crates/anvil/src/main.rs` — construct a second `Webview` for chrome
  alongside the palette one (near line 3377); add `chrome_dirty` to
  `App`; flush in `AppShell::tick`; gate the Metal `draw_status_bar`
  call (`main.rs:1509`) behind `chrome_overlay_active` so we can
  toggle off if the spike fails.
- `crates/anvil-render/src/statusbar.rs` — kept as fallback; deleted
  in a follow-up only after the spike sticks.

## Phase 1 Task List

1. Refactor `appkit.rs` to add `AnvilContainerView` as the content view
   with terminal as child.
   → verify: app launches, terminal renders, palette opens, no regression.
2. Add `persistent: bool` to `WebviewConfig`; palette passes `false`.
   → verify: `cargo test --workspace` passes; `Cmd-P` still works.
3. Add `ui/chrome/index.html` with a fixed-height bottom strip,
   `pointer-events: none` on body, `auto` on the strip, placeholder text.
   → verify: open in Safari, strip renders.
4. Construct a second `Webview` in `main.rs` for chrome, persistent,
   full-window, loading `ui/chrome/index.html`.
   → verify: launch app — strip visible at bottom, terminal types and
     scrolls underneath, nothing grabs focus.
5. Add `Outbound::ChromeState` and `Inbound::Chrome` variants with tests.
   → verify: `cargo test -p anvil-control`.
6. Add `chrome_dirty` on `App`; set on cwd/clock change; flush in `tick`.
   HTML renders `status.cwd` and `status.clock`.
   → verify: launch, `cd` updates cwd, clock advances each minute.
7. Hit-testing smoke: type into terminal works; click status strip's
   palette hot-zone routes `OpenPalette` and shows palette.
   → verify: manual.
8. Resize: drag through several sizes; no new black-flash regressions;
   strip stays bottom-anchored.
   → verify: manual.

## Verification / Discard Test

**Spike succeeds if:**

- Status bar renders via HTML/CSS, layered over Metal, no visual
  regression vs today.
- Terminal typing/scrolling unchanged.
- Editing `ui/chrome/index.html` CSS and reloading the WebView changes
  the strip **without a Rust rebuild**.
- `cwd` and `clock` update from Rust state.
- Live resize introduces no new black-screen failure mode.

**Spike fails (revert) if:**

- Click-through on transparent regions is unreliable (typing or
  selection breaks intermittently).
- Live resize visibly tears or drifts >1 frame between layers.

Sharpest single observable: **CSS-only edits to the status bar change
pixels on screen after a WebView reload, with no `cargo build`.** If
that works, the model is proved and we extend to the tab bar in phase 2.
If it doesn't, we revert.

## Out of Scope

- Tab bar HTML port (phase 2).
- Block headers, agent panel, palette consolidation onto this WebView
  (phase 3+).
- Deleting `statusbar.rs` / `tabbar.rs` (only after phases stick).
- Theme push over IPC (phase 1 hard-codes Mineral colors in CSS).
- Fixing the existing black-screen-on-resize bug (orthogonal).
