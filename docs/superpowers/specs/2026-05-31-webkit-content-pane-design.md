# WebKit Content-Pane Foundation — Design

**Date:** 2026-05-31
**Status:** Approved (brainstorm)
**Amends:** `wiki/decisions/0005-render-host.md`
**Scope:** Sub-project #1 of the hybrid-architecture spine. Foundation only.

## Goal

Render a live `WKWebView` inside a real Anvil pane as a navigable docs/browser
surface, with a thin native↔web bridge. This is the load-bearing primitive that
later unlocks the extension model (sub-project #2) and concrete extensions —
k8s/CI/IaC/observability (sub-project #3). Those are **out of scope here.**

## Relationship to decision 0005

0005 ("Render Host: Native Metal, No Webview Chrome") already permits a WKWebView
**only as the content of a dedicated pane type**, never as host chrome, and
defers the typed native↔web bridge until such a pane exists. This spec activates
that escape hatch. It does **not** reverse 0005: app chrome stays native Metal.
A follow-up decision record amends 0005 to note the content pane now exists.

## Context: the compositing constraint

Anvil composites everything through one `CAMetalLayer` (chrome + terminal cells
share `anvil_frame`, one atlas, one draw loop). A `WKWebView` is an AppKit
`NSView`; it cannot be cheaply rendered into the Metal frame. Therefore a web
pane is a **real NSView subview layered into the NSWindow**, positioned and
clipped to the pane rect by the Obj-C shim, hidden when the pane is off-screen.
Metal chrome and webview panes are sibling layers.

**Hard consequence:** no Metal chrome can float *over* web content. The pane's
URL bar lives in a reserved **header strip above** the web NSView, never as an
overlay. This matches cmux's libghostty-Metal + WKWebView model.

## Architecture & module boundaries

### `src/web/pane.zig` (new)

`WebPane` — the testable logic unit.

```zig
pub const WebPane = struct {
    handle: ?*anyopaque = null, // opaque WKWebView*, owned by the shim
    url: [2048]u8 = undefined,
    url_len: usize = 0,
    title: [256]u8 = undefined,
    title_len: usize = 0,
    loading: bool = false,
    progress: f32 = 0, // estimatedProgress, 0..1
    can_back: bool = false,
    can_fwd: bool = false,
};
```

Responsibilities: own the handle, hold mirrored state, and call the C-ABI shim
(`create/navigate/back/forward/reload/setFrame/setHidden/destroy`). Geometry math
(pane rect → web body frame) lives here. Shim calls go through a function table
(`Backend`) so headless tests can stub them.

```zig
pub const Backend = struct {
    create: *const fn () callconv(.c) ?*anyopaque,
    navigate: *const fn (?*anyopaque, [*:0]const u8) callconv(.c) void,
    back: *const fn (?*anyopaque) callconv(.c) void,
    forward: *const fn (?*anyopaque) callconv(.c) void,
    reload: *const fn (?*anyopaque) callconv(.c) void,
    set_frame: *const fn (?*anyopaque, f64, f64, f64, f64) callconv(.c) void,
    set_hidden: *const fn (?*anyopaque, bool) callconv(.c) void,
    destroy: *const fn (?*anyopaque) callconv(.c) void,
};
```

The default backend wires to the real `anvil_web_*` shim exports; tests inject a
recording stub.

`headerStripHeight` is a const; `bodyFrame(pane_rect)` returns the rect minus the
strip. `setFrame(pane_rect, visible)` computes the body frame and forwards to the
backend, or `set_hidden(true)` when `!visible`.

### `src/platform/shim.m` (extend)

New Obj-C, new C exports:

- `anvil_web_create() -> void*` — build a `WKWebView` with an **ephemeral**
  `WKWebsiteDataStore` (no shared cookies/creds, no cross-session persistence),
  add as an NSWindow subview, register KVO on `title`, `estimatedProgress`,
  `canGoBack`, `canGoForward`. Inject a `documentStart` userscript that sets
  Mineral theme CSS vars (best-effort; sites may ignore).
- `anvil_web_navigate(void*, const char* url)` — `loadRequest`. Block non-`http(s)`
  schemes from the URL bar; reject remote `file://`.
- `anvil_web_back/forward/reload(void*)`.
- `anvil_web_set_frame(void*, x, y, w, h)` — position/size the subview (window
  coords; shim flips to AppKit's bottom-left origin).
- `anvil_web_set_hidden(void*, bool)`.
- `anvil_web_destroy(void*)` — remove subview, release.
- `anvil_web_set_callback(fn)` — register one Zig callback the KVO observers and
  the content-process-terminated delegate invoke with `(handle, event, payload)`
  so the matching `WebPane` updates `title`/`progress`/`can_back`/`can_fwd`/
  `loading`/`failed`.

The `WKWebViewWebContentProcessDidTerminate` delegate auto-reloads once, then
reports a failed state.

### `src/session.zig` (extend)

Follow the existing `editor: ?Editor` pattern:

- `Kind = enum { shell, viewer, editor, web }`.
- `web: ?WebPane = null`.
- `initWeb(alloc, rows, cols, url)` mirrors `initEditor`: build the (unused-body)
  Terminal for the header strip, create the WebPane, navigate to `url`.
- `deinit` `.web` branch destroys the WebPane.
- `resize` `.web` branch is a no-op on the grid; app.zig drives the NSView frame.
- `poll` returns `{ alive = true, consumed = false }` for `.web` (no pty).

The Terminal is allocated but its body is unused (only the header strip is
chrome). Minor waste, accepted for v1 to keep the pane-id→session map uniform.

### `src/app.zig` (wire)

- After computing pane rects each frame, for `.web` sessions call
  `WebPane.setFrame(rect, visible)` where `visible` = pane is the active tab and
  not hidden by zoom.
- Draw the header strip (back/fwd/reload/url/progress) through the existing chrome
  pipeline, in the reserved band atop the web body.
- Route `Cmd+L` → focus URL bar (Anvil chrome input); `Enter` → `navigate`;
  `Esc` → return first responder to the Metal view.
- Pane close / window close → destroy web panes.

### `pane_tree.zig` — unchanged

Topology/geometry only; content kind lives in Session.

## Bridge (thin, v1)

- **native→web:** `navigate`, `back`, `forward`, `reload`; theme CSS vars via the
  documentStart userscript.
- **web→native:** KVO only — `title`, `estimatedProgress`, `canGoBack/Forward`,
  plus the content-process-terminated delegate. **No custom
  `WKScriptMessageHandler` actions** — that is sub-project #2's extension bridge.

## Header strip (native Metal chrome)

Reserved band atop the web NSView, existing chrome pipeline, Mineral tokens:

- back / forward / reload buttons (dim when `!can_back` / `!can_fwd`).
- URL text field — `Cmd+L` focuses, `Enter` navigates.
- loading progress — thin line driven by `estimatedProgress`.
- styled to match the existing pane header strip.

## Invocation

v1 entry points: command-palette action "Open browser pane" and a keybind, both
opening a split with a `.web` session (blank/start URL). The `anvil` CLI/IPC
`open --web <url>` verb is **deferred** to a later slice.

## Error handling

- Nav failure → WKWebView's own error page; header shows failed state; no Anvil
  crash path.
- Web content process crash → delegate auto-reloads once, else failed state.

## Security guardrails

- Ephemeral `WKWebsiteDataStore`: no cookies/creds shared with native, no
  cross-session persistence.
- No Anvil secrets/env injected into page JS; userscript injects only theme CSS
  vars.
- Block non-`http(s)` schemes and remote `file://` from the URL bar.
- WKWebView's out-of-process renderer is the sandbox boundary.
- URL bar is the **only** navigation source in v1. No auto-navigation from
  observed page content or agent output (that needs explicit later design).

## Testing

- **Zig unit tests** (`src/web/pane.zig`) with a stub `Backend`:
  - state machine: `navigate` → `loading=true`/`progress` advances → callback sets
    `title`, `loading=false`, `can_back/fwd`.
  - geometry: `bodyFrame(rect)` = rect minus header strip; `setFrame` forwards the
    body frame when visible, calls `set_hidden(true)` when not.
  - the stub records calls; assertions check the call sequence + args.
- **Manual / live:** run the app — open pane, navigate, resize, split-resize,
  tab-switch (hide/show), close.
- **Known limit:** `--dump` is Metal-only; it captures the header strip (in the
  Metal frame) but **not** the web body (separate NSView). Documented, not a
  regression.

## Out of scope (later sub-projects / slices)

- Custom `WKScriptMessageHandler` native↔web actions (extension bridge → #2).
- Extension definition / isolation / capabilities (#2).
- Concrete k8s/CI/IaC/observability extensions (#3).
- Session-restore persistence of web panes.
- `anvil open --web <url>` CLI/IPC verb.
- Auto-navigation from agent output or page content.

## Persistence note

v1 does **not** reopen web panes on session restore (`session_persist.zig`
untouched for `.web`). Deferred deliberately.
