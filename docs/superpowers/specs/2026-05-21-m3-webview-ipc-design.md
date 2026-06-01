# M3 — Webview Host, Typed IPC Bridge, Command Palette — Design Spec

> Status: draft for review. Milestone **M3**.
> Created: 2026-05-21. Owner-approved direction; pending spec review.

## Goal

Make Anvil a **hybrid app**: embed a webview in the native window,
build a typed native↔web IPC bridge, and ship the first web surface — a
**command palette** overlay.

M3 is the architectural pivot. Until now the app is purely native (Zig + Metal
terminal). M3's job is not primarily to ship a feature — it is to **de-risk the
rest of the roadmap**: M4 (browser), M5 (editor), M6 (agents), and M7 (plugins)
are all web surfaces hosted by the machinery built here. The command palette is
the proof vehicle: small enough to land in one milestone, real enough to
exercise an in-window webview and bidirectional IPC.

## Context

The hybrid architecture (Option B in `docs/product/console-rebuild-plan.md`) is
already decided: the terminal stays native Zig + Metal; every other surface is
web UI in an embedded webview. M3 is the first milestone to act on that.

Current interop pattern (verified against the codebase at this commit): the
project uses the `zig-objc` package. Runtime Objective-C classes are built with
`objc.allocateClassPair` + `Object.addMethod` + `objc.registerClassPair` —
that is how `CalderaDelegate` (app lifecycle, resize, 60 Hz `tick:`) and
`CalderaTerminalView` (`keyDown:`, `scrollWheel:`) are constructed in
`src/main.zig`. There are no `.m` glue files. The window's content view is the
layer-backed `CalderaTerminalView` carrying a `CAMetalLayer`. Assets are
embedded into the binary (`src/assets/app-icon.png`); Metal shaders compile at
runtime — the project favors a single self-contained binary with no offline
toolchain. M3 follows all of these patterns.

## Decisions (settled with the owner)

1. **First surface** — a command palette. It exercises bidirectional IPC
   (native pushes the command list; web sends back the chosen command; native
   acts) and is the UX concept carried over from `anvil-console`.
2. **Compositing model — Approach A: full-window transparent overlay webview.**
   One `WKWebView` fills the window content area, layered above the Metal
   terminal view, transparent, hidden by default. Rejected: a panel-sized
   webview (does not generalize to M4's browser; re-does layout work per
   surface) and a separate floating `NSPanel` window (sidesteps the in-window
   webview risk that M3 exists to retire; cannot host M4/M5).
3. **No surface manager yet.** The plan's target architecture names a
   `src/surfaces/` surface-manager. M3 has exactly one terminal and one
   overlay; a generic manager now is a speculative abstraction. `surfaces/`
   arrives at M4+ when there is a second real surface to manage.
4. **Webview library** — `WKWebView` directly via `zig-objc`. Rejected the
   cross-platform `webview` C library: macOS-only app, adds a dependency, and
   the `zig-objc` path matches existing AppKit/Metal interop.
5. **Web stack** — one self-contained `index.html` (inline CSS + JS), no
   framework, no build step, no node. A framework + build pipeline is an
   M4/M5 decision, deferred (YAGNI).
6. **Asset delivery** — the web surface is embedded into the Zig binary at
   build time and loaded with `loadHTMLString:`, matching the existing
   `@embedFile` asset pattern and the single-binary ethos.

## Architecture

Three new modules plus one web surface, wired together in `src/main.zig`.

| Path | Responsibility | ObjC-bound | Unit-tested |
|---|---|---|---|
| `src/webview/webview.zig` | `WKWebView` host: create it, add as a hidden subview above the Metal view; the `WKScriptMessageHandler` delegate class; `show` / `hide`; `evalJS`; load the embedded HTML | yes | no |
| `src/ipc/bridge.zig` | The typed message protocol: `Inbound` / `Outbound` tagged unions, JSON encode/decode, dispatch | no | yes — fully |
| `src/app/palette.zig` | Command-palette controller: the command catalog, summon/dismiss state, invoked-id → action mapping | no | yes |
| `ui/palette/index.html` | The web surface — self-contained HTML + CSS + JS | — | manual |

`webview.zig` and `ipc/bridge.zig` are unit-test-free or fully unit-tested by
the same rule the codebase already follows: ObjC-bound code (`metal.zig`,
`pty.zig`) is verified manually; pure logic is unit-tested.

### Webview host (`src/webview/webview.zig`)

- After the window and `CalderaTerminalView` are created in `main()`, create
  one `WKWebView` sized to the content rect. Make it non-opaque
  (`setOpaque:false`, `drawsBackground` = NO) so the web-drawn dim backdrop
  shows through to the terminal.
- Add it as a **subview of the terminal view**. Its Core Animation layer then
  composites above the `CAMetalLayer` automatically — no shared Metal/GL
  context. The terminal keeps presenting drawables on the 60 Hz tick,
  untouched.
- It starts **`setHidden:true`**. A hidden `NSView` draws nothing and receives
  no events, so the terminal owns input by default.
- **Summon:** `setHidden:false`; `window.makeFirstResponder:` the webview;
  send `show`. **Dismiss:** `setHidden:true`; `makeFirstResponder:` back to the
  terminal view.
- The `WKScriptMessageHandler` is a runtime ObjC class `CalderaScriptHandler`
  built with `objc.allocateClassPair(NSObject, ...)` +
  `addMethod("userContentController:didReceiveScriptMessage:", ...)` +
  `registerClassPair` — identical in shape to `CalderaDelegate`. It is
  registered on the webview's `WKUserContentController` under the name
  `caldera`.

### Typed IPC bridge (`src/ipc/bridge.zig`)

**Transport.** JSON strings both directions:

- web → native: `window.webkit.messageHandlers.caldera.postMessage(json)`
- native → web: `evaluateJavaScript("window.caldera.receive(json)")`

**Protocol.** One module owns the whole message catalog — no stringly-typed
handling scattered across the codebase.

```
Outbound (native → web):
  .show   { commands: []Command, theme: ThemeTokens }
  .hide   {}

Inbound (web → native):
  .ready    {}                    // webview finished loading — handshake
  .invoke   { id: []const u8 }     // user chose a command
  .dismiss  {}                     // Esc pressed or backdrop clicked

Command     = { id: []const u8, title: []const u8, subtitle: ?[]const u8 }
ThemeTokens = { background, foreground, accent } — the active theme colors
              the palette needs to match the terminal, as hex strings
```

`bridge.zig` exposes `encode(Outbound) -> []u8` and `decode([]u8) -> !Inbound`.
The ObjC message handler in `webview.zig` hands the raw string to `decode` and
routes the typed result. Five message types, bidirectional.

### Command palette (`src/app/palette.zig`)

A static **command catalog** of the real native actions available at M3:

| id | action |
|---|---|
| `theme.dark` | switch the active theme to `mineral-dark` |
| `theme.light` | switch the active theme to `mineral-light` |
| `config.reload` | re-read the config file |
| `terminal.clear` | clear the visible screen (`ESC[H ESC[2J`) — scrollback is kept |
| `scroll.top` | scroll to the top of scrollback |
| `scroll.bottom` | scroll to the bottom (live edge) |
| `app.quit` | quit the app |

`palette.zig` holds the catalog, the summon/dismiss state, and an
`invoke(id) -> Action` mapping. It does not touch ObjC. The catalog is
thin on purpose: every M2/M4+ feature later registers into this same catalog.

**Summon keybind:** ⌘K — the modern command-palette convention, and free (the
terminal binds no ⌘K). It is intercepted in the terminal view's `keyDown:`
before PTY byte-encoding. **Dismiss:** Esc, handled entirely web-side — while
the palette is open the webview holds first-responder, so Esc never reaches the
terminal's `keyDown:`; the web UI posts `dismiss`. Hardcoded for M3; routing
summon through the config keybinding system is later work.

### Web UI (`ui/palette/index.html`)

One self-contained file: inline CSS and JS, no framework, no build step. It is
embedded into the Zig binary at build time (a build-system anonymous import)
and loaded via `loadHTMLString:`.

Behavior: on `show` it renders the command list and fuzzy-filters as the user
types; Enter sends `invoke { id }`; Esc / backdrop click sends `dismiss`. On
load it sends `ready`.

Styling follows `BRAND.md`: Mineral palette, IBM Plex type, a dim graphite
backdrop, semantic status colors. The palette uses the `theme` tokens from the
`show` message so the overlay matches the active terminal theme.

## Data flow

**Summon.** ⌘K in `keyDown:` → `palette.zig` builds the catalog → `main.zig`
shows the webview and sends `show { commands, theme }` via `bridge.encode` +
`webview.evalJS`. If the webview has not yet sent `ready`, the `show` is
deferred until `ready` arrives.

**Invoke.** User picks a command → web posts `invoke { id }` →
`CalderaScriptHandler` → `bridge.decode` → `palette.invoke(id)` → `main.zig`
runs the action and hides the webview.

**Dismiss.** Esc or backdrop click → web posts `dismiss` → handler →
`bridge.decode` → `main.zig` hides the webview and restores first-responder to
the terminal view.

## Error handling

- Malformed JSON, unknown message type, unknown command id → **log and drop,
  never crash**. `decode` returns a Zig error; the handler logs `@errorName`
  and returns.
- Webview HTML fails to load → log; the palette simply will not summon and the
  terminal keeps working (graceful degradation).
- `show` sent before `ready` → deferred and flushed on `ready`, not lost.

## The hard part — webview / Metal coexistence

M3's real risk is an in-window webview coexisting with the Metal terminal. The
design names every sharp edge so the implementation plan can test each:

1. The webview must start `setHidden:true`, or it steals first-responder.
2. First-responder must be moved explicitly on summon **and** dismiss.
3. The webview must be non-opaque with no backdrop, or it white-flashes over
   the terminal.
4. `windowDidResize:` must resize the webview frame alongside the Metal layer.
5. The `ready` handshake must gate `show`, or an early summon is lost.

## Testing

- `src/ipc/bridge.zig` — unit tests: round-trip encode/decode of every message
  type; malformed JSON; unknown message type.
- `src/app/palette.zig` — unit tests: catalog lookup; id → action mapping;
  summon/dismiss state transitions.
- `src/webview/webview.zig` — no unit tests (ObjC-bound, consistent with
  `metal.zig` / `pty.zig`); verified manually.
- Manual verification (`zig build run`): ⌘K summons the palette over a
  still-rendering terminal → typing filters the list → Enter switches the theme
  for real → Esc dismisses and focus returns to the terminal → window resize
  keeps the overlay aligned.

## Non-goals (deferred past M3)

- A generic surface manager (`src/surfaces/`) — M4+.
- A web build pipeline / framework — M4/M5.
- Configurable keybindings for summon/dismiss — later.
- Any web surface other than the command palette — M4 onward.
- Commands beyond the M3 catalog — features register as they are built.

## Dependencies / sequencing

M3 depends on nothing in M2's unbuilt sub-projects (multi-tab, search, shell
integration). Per owner direction, M3 is **planned now but implemented after
M2 merges** to avoid branch churn against in-flight M2 work.
