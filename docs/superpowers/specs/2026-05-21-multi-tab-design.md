# Multi-Tab — Design Spec

> Status: draft for review. Sub-project 2 of 4 in milestone **M2**.
> Created: 2026-05-21. Owner-approved direction via brainstorm answers.

## Goal

Let Anvil hold multiple terminal tabs in one window — each tab its own
shell — with a low-profile, themed tab bar and standard macOS keyboard shortcuts.

## Context

After M2 sub-project 1, the app is a single Metal view driven by one global
`App` holding one `Terminal` + one `Pty`. The PTY→main-thread handoff uses
module-level globals (`pty_buf`, `pty_len`, `pty_mutex`, `pty_dead`,
`feed_scratch`) and one reader thread (`ptyReaderThread`). Multi-tab makes that
per-tab.

## Decisions (settled with the owner)

1. **Tab bar** — custom-drawn into the Metal raster (not AppKit chrome), themed
   with the Mineral palette.
2. **Low profile** — the bar is *hidden entirely* with one tab (the terminal
   looks exactly like today); it appears only at 2+ tabs.
3. **Keybindings** — standard macOS set, **configurable in `config.zon`** and
   live-reloadable: `⌘T` new, `⌘W` close, `⌘⇧[` / `⌘⇧]` prev/next, `⌘1`–`⌘9`
   jump to tab N.
4. **Tab label** — the shell-set title (OSC 0/2); if empty, the cwd's last path
   component (OSC 7); if empty, `"shell"`.
5. **PTY threading** — one reader thread + one handoff buffer *per tab*
   (Approach A). Background tabs stay live; `onTick` drains every tab.

## Architecture

```
src/app/tab.zig       Tab (Terminal + Pty + per-tab PTY handoff + reader thread)
                      and TabManager (the tab list, active index, lifecycle)
src/render/tabbar.zig drawTabBar — the thin themed bar, drawn into the raster
src/config/config.zig + a `keybindings` section (live-reloadable)
src/main.zig          wires TabManager; routes input; drains all tabs each tick;
                      draws the active tab + (at 2+ tabs) the bar
```

### `Tab`

Each tab is **heap-allocated** so its address is stable — the reader thread
holds a `*Tab`, and tabs opening/closing must not move other tabs.

```zig
pub const Tab = struct {
    alloc: std.mem.Allocator,
    terminal: Terminal,
    pty: Pty,

    // PTY -> main-thread handoff (was module-global in M1; now per-tab).
    buf: [256 * 1024]u8 = undefined,
    len: usize = 0,
    mutex: std.Thread.Mutex = .{},
    dead: bool = false,
    reader: ?std.Thread = null,

    pub fn create(alloc, cols, rows, scrollback_capacity, cwd: ?[]const u8) !*Tab;
    pub fn deinit(self: *Tab) void; // close pty -> reader thread exits -> join -> free
    pub fn label(self: *const Tab, buf: []u8) []const u8; // title -> cwd base -> "shell"
};
```

The handoff buffer shrinks from M1's 1 MiB to 256 KiB — adequate per tab, and N
tabs should not cost N MiB. The reader-thread function takes a `*Tab` and uses
the tab's own `buf`/`mutex`/`dead` instead of globals.

### `TabManager`

```zig
pub const TabManager = struct {
    alloc: std.mem.Allocator,
    tabs: std.ArrayList(*Tab),
    active: usize,

    pub fn init(alloc) TabManager;
    pub fn deinit(self: *TabManager) void;        // deinit + free every tab
    pub fn current(self: *TabManager) *Tab;
    pub fn newTab(self, cols, rows, scrollback, cwd) !void;  // append, make active
    pub fn closeActive(self) void;                // deinit, remove, fix `active`
    pub fn switchTo(self, index: usize) void;     // clamp to range
    pub fn next(self) void; pub fn prev(self) void;  // wrap around
    pub fn barVisible(self) bool;                 // tabs.len > 1
};
```

`closeActive` deinits the active tab, removes it from the list, and adjusts
`active` (when the last tab in the list closes, `active` steps back). When the
list becomes empty the app terminates the window.

## Data flow

**New tab** (`⌘T`): `TabManager.newTab` creates a `Tab` whose shell spawns in the
current tab's cwd (from OSC 7) when known, else the inherited default. The new
tab becomes active. If this is the 1→2 transition the bar appears, so every
tab's grid + PTY shrink by one text row (see "Bar show/hide").

**Per tick** (`onTick`): drain *every* tab's handoff buffer into that tab's
`Terminal` (background tabs stay current); render only the active tab; if a
tab's `dead` flag is set, close that tab.

**Render**: draw the active tab's terminal grid; if `barVisible()`, draw the tab
bar into the top row via `tabbar.drawTabBar`.

**Switch** (`⌘⇧[`/`]`, `⌘1`–`9`, bar click): change `active`, mark dirty.

**Close** (`⌘W`): `closeActive`; if no tabs remain, terminate.

## Bar show/hide and grid sizing

The bar occupies exactly one text row (`font.metrics.cell_h` device px) at the
top. The terminal grid height is `total_rows - (if barVisible 1 else 0)`. The
1↔2-tab transitions change bar visibility, so on every `newTab` and `closeActive`
that crosses the boundary, **all** tabs' `Terminal.resize` and `Pty.resize` run
with the new row count. `onResize` (window resize) likewise sizes every tab.

## Input

- `keyDown`: before encoding bytes for the active terminal, match the event
  against the configured tab keybindings. A match runs the tab action and
  consumes the event; otherwise bytes go to `current().terminal`/`pty`.
- The current `extractKey` returns null for any `⌘` combo. Tab shortcuts are
  matched from the raw event (modifier flags + keycode/character) *before* that
  early return.
- New `mouseDown:` handler on the view: when the bar is visible and the click
  is in the top row, hit-test which tab segment was hit and `switchTo` it.

## Config — keybindings

`config.zig` gains a `keybindings` struct on `Config`; chord strings parsed into
matchable bindings. Defaults are the standard set.

```zon
.keybindings = .{
    .new_tab = "cmd+t",
    .close_tab = "cmd+w",
    .next_tab = "cmd+shift+]",
    .prev_tab = "cmd+shift+[",
    // tab_1 .. tab_9 default to "cmd+1" .. "cmd+9"
},
```

A chord string is `[mod+]*key` where mods are `cmd`/`shift`/`ctrl`/`opt` and key
is a single character or a named key. Parsing is pure and unit-tested. Invalid
chords log and fall back to the default for that action. Live-reloadable through
the existing `Watcher` (rebind without restart).

## Error handling

| Situation | Behavior |
|---|---|
| `newTab` fails to spawn a PTY/terminal | Log to stderr; keep the current tabs unchanged; do not crash. |
| `closeActive` on the last tab | Terminate the app (window closes). |
| A tab's shell exits (`dead`) | That tab auto-closes on the next tick; if it was the last, the app terminates. |
| Invalid keybinding chord in config | Log; that action keeps its default binding. |
| More than a sane max of tabs (e.g. 32) | `newTab` is a no-op with a logged note — bounds the per-tab thread/buffer cost. |

## Testing

`zig build test` is the gate; all sub-project-1 tests stay green.

Spawning a real shell per tab in unit tests would be slow and flaky, so the
**index/lifecycle bookkeeping is extracted into pure functions** that are tested
directly, while real-PTY behavior is integration-verified.

**`tab.zig` — pure helpers (unit-tested):**
- `nextActiveAfterClose(count, closed_index, active) usize` — the index a
  manager should land on after closing a tab. Cases: close the active first /
  middle / last tab; close a non-active tab below/above the active one.
- `wrapIndex(count, index, delta) usize` — wrap-around for `next`/`prev`.
- `clampIndex(count, index) usize` — for `switchTo` out-of-range input.
- `barVisible(count) bool` — false at `count <= 1`, true at `count >= 2`.
- `Tab.label` operating on supplied title/cwd byte buffers — resolves
  title → cwd basename → `"shell"` (test each fallback; test cwd basename
  extraction including a trailing-slash cwd).

`TabManager.newTab`/`closeActive`/`next`/`prev`/`switchTo` are thin wrappers
that call these helpers, so testing the helpers covers the logic. Full `Tab`
lifecycle with a real `Pty` (spawn, reader thread, join on close) is
integration-verified, consistent with how M1's `pty.zig` is exercised.

**`config.zig`** — keybinding chord parsing: valid chords, every modifier,
named keys, invalid chord → default for that action.

**`tabbar.zig`** — `drawTabBar` paints the bar row (a pixel probe like the
existing raster tests) and the active segment uses the accent color.

The PTY-per-tab threading and AppKit mouse wiring are integration-verified
(`zig build run`): open tabs with `⌘T`, switch, close, click the bar.

## Out of scope (deliberate)

- Tab reordering by drag.
- Tab-bar overflow/scrolling when tabs exceed the window width (labels just
  shrink/elide; a hard tab cap bounds it).
- Per-tab split panes.
- Persisting/restoring tabs across launches.
- Tab close buttons in the bar (close is `⌘W` / shell exit) — keeps the bar
  low-profile; revisit later if wanted.

## File summary

| File | Change |
|---|---|
| `src/app/tab.zig` | Create — `Tab`, `TabManager`, the per-tab reader thread. |
| `src/render/tabbar.zig` | Create — `drawTabBar`. |
| `src/config/config.zig` | Modify — add the `keybindings` struct + chord parsing. |
| `src/main.zig` | Modify — `TabManager` integration; input routing; per-tab tick drain; mouse handler; render the bar. Removes the M1 module-global PTY handoff. |
| `src/pty/pty.zig` | Unchanged (already supports per-instance spawn/read/write/resize). |
| `src/terminal/terminal.zig` | Unchanged (already exposes `title`/`cwd`). |
