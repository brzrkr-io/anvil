# WebKit Content-Pane Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a live, navigable `WKWebView` inside an Anvil pane (docs/browser surface) with a thin native↔web bridge.

**Architecture:** A web pane is a real AppKit `NSView` subview layered into the NSWindow (not a Metal draw), positioned/clipped to the pane rect by the Obj-C shim. Pure display state (`WebPane`) lives in the root Zig module and is unit-tested; the native `WKWebView` handle and all extern shim calls live in `app.zig` (the native boundary), which reconciles handle lifecycle against live `.web` sessions each frame. The pane's URL bar is native Metal chrome in a reserved header strip above the web NSView.

**Tech Stack:** Zig 0.16, AppKit/WebKit (Obj-C shim), Metal chrome. Compiler `.zig/zig`.

**Spec:** `docs/superpowers/specs/2026-05-31-webkit-content-pane-design.md`

### Deviation from spec (intentional)

The spec proposed a `Backend` function-table on `WebPane` for test injection. This plan drops it. Reason: `mod_tests` (the root module test binary) does not link `shim.m`, so any extern Obj-C reference in the root module breaks the test link. Instead `WebPane` is pure (no externs, fully testable directly), and `app.zig` — which is in `exe.root_module` and *does* link the shim — owns the native handle and makes every extern call. This is simpler and preserves the spec's testability intent.

### File structure

- **Create `src/web/pane.zig`** — `WebPane`: pure display state (url/title/loading/progress/can_back/can_fwd/failed) + pure helpers (`bodyFrame`, `validateUrl`, state setters). No extern calls. Unit-tested via root module.
- **Modify `src/session.zig`** — add `Kind.web` + `web: ?WebPane`, `initWeb`, and `.web` branches in `deinit`/`resize`/`poll`.
- **Modify `src/session_manager.zig`** — `addWeb(url, rows, cols)`, mirroring `addEditor`.
- **Modify `src/root.zig`** — register `web/pane.zig` in the test aggregator.
- **Modify `src/platform/shim.m`** — `WKWebView` lifecycle C functions (`anvil_web_*`), KVO + crash delegate calling back into Zig.
- **Modify `build.zig`** — `linkFramework("WebKit")`.
- **Modify `src/app.zig`** — native handle map + per-frame reconcile/layout, `anvil_web_event` export, header-strip URL chrome + input/focus routing, palette/keybind invocation.

---

## Task 1: `WebPane` pure module

**Files:**
- Create: `src/web/pane.zig`
- Modify: `src/root.zig` (test aggregator)

- [ ] **Step 1: Write the failing tests**

Create `src/web/pane.zig` with the struct and tests first (implementation stubbed so it compiles-fails on assertions):

```zig
const std = @import("std");

/// Pure display state for one web (WKWebView) pane. Holds no native handle and
/// makes no platform calls — app.zig owns the WKWebView and drives this state.
/// Reserved native-chrome strip above the web body, in points.
pub const header_strip_h: f64 = 30;

pub const Rect = struct { x: f64, y: f64, w: f64, h: f64 };

pub const WebPane = struct {
    url_buf: [2048]u8 = undefined,
    url_len: usize = 0,
    title_buf: [256]u8 = undefined,
    title_len: usize = 0,
    loading: bool = false,
    progress: f32 = 0,
    can_back: bool = false,
    can_fwd: bool = false,
    failed: bool = false,

    pub fn init(url: []const u8) WebPane {
        var w = WebPane{};
        w.setUrl(url);
        return w;
    }

    pub fn url(self: *const WebPane) []const u8 {
        return self.url_buf[0..self.url_len];
    }

    pub fn title(self: *const WebPane) []const u8 {
        return self.title_buf[0..self.title_len];
    }

    pub fn setUrl(self: *WebPane, s: []const u8) void {
        const n = @min(s.len, self.url_buf.len);
        @memcpy(self.url_buf[0..n], s[0..n]);
        self.url_len = n;
    }

    pub fn setTitle(self: *WebPane, s: []const u8) void {
        const n = @min(s.len, self.title_buf.len);
        @memcpy(self.title_buf[0..n], s[0..n]);
        self.title_len = n;
    }

    /// A navigation started: loading, not failed, progress reset.
    pub fn beginNav(self: *WebPane, s: []const u8) void {
        self.setUrl(s);
        self.loading = true;
        self.failed = false;
        self.progress = 0;
    }

    pub fn setProgress(self: *WebPane, p: f32) void {
        self.progress = p;
        if (p >= 1.0) self.loading = false;
    }

    pub fn setNav(self: *WebPane, back: bool, fwd: bool) void {
        self.can_back = back;
        self.can_fwd = fwd;
    }

    pub fn setFailed(self: *WebPane) void {
        self.failed = true;
        self.loading = false;
    }
};

/// The web body occupies the pane rect minus the header strip at the top.
/// Returned in the same coordinate space as `pane`.
pub fn bodyFrame(pane: Rect) Rect {
    const h = @max(pane.h - header_strip_h, 0);
    return .{ .x = pane.x, .y = pane.y + header_strip_h, .w = pane.w, .h = h };
}

/// True only for http/https. The URL bar is the sole nav source; everything
/// else (file://, javascript:, data:, custom schemes) is rejected.
pub fn validateUrl(s: []const u8) bool {
    return std.ascii.startsWithIgnoreCase(s, "http://") or
        std.ascii.startsWithIgnoreCase(s, "https://");
}

test "init stores url, empty title, not loading" {
    const w = WebPane.init("https://example.com");
    try std.testing.expectEqualStrings("https://example.com", w.url());
    try std.testing.expectEqualStrings("", w.title());
    try std.testing.expect(!w.loading);
}

test "beginNav sets loading and clears failed; progress completes it" {
    var w = WebPane.init("https://a.test");
    w.setFailed();
    try std.testing.expect(w.failed);
    w.beginNav("https://b.test");
    try std.testing.expectEqualStrings("https://b.test", w.url());
    try std.testing.expect(w.loading and !w.failed);
    w.setProgress(0.5);
    try std.testing.expect(w.loading);
    w.setProgress(1.0);
    try std.testing.expect(!w.loading);
}

test "setNav and setTitle mutate state" {
    var w = WebPane.init("https://a.test");
    w.setNav(true, false);
    w.setTitle("Example");
    try std.testing.expect(w.can_back and !w.can_fwd);
    try std.testing.expectEqualStrings("Example", w.title());
}

test "bodyFrame subtracts the header strip from the top" {
    const b = bodyFrame(.{ .x = 10, .y = 20, .w = 800, .h = 600 });
    try std.testing.expectEqual(@as(f64, 10), b.x);
    try std.testing.expectEqual(@as(f64, 20 + header_strip_h), b.y);
    try std.testing.expectEqual(@as(f64, 800), b.w);
    try std.testing.expectEqual(@as(f64, 600 - header_strip_h), b.h);
}

test "validateUrl accepts http/https only" {
    try std.testing.expect(validateUrl("https://x.test"));
    try std.testing.expect(validateUrl("HTTP://x.test"));
    try std.testing.expect(!validateUrl("file:///etc/passwd"));
    try std.testing.expect(!validateUrl("javascript:alert(1)"));
    try std.testing.expect(!validateUrl("x.test"));
}
```

- [ ] **Step 2: Register tests in the root aggregator**

In `src/root.zig`, inside the `test { ... }` block, after the `_ = @import("editor.zig");` line, add:

```zig
    _ = @import("web/pane.zig");
```

- [ ] **Step 3: Run the tests**

Run: `.zig/zig build test 2>&1 | tail -20`
Expected: PASS (all `web/pane.zig` tests green; the file already contains the full implementation above, so they pass on first run).

- [ ] **Step 4: Format**

Run: `.zig/zig fmt src build.zig`
Expected: no diff / clean.

- [ ] **Step 5: Commit**

```bash
git add src/web/pane.zig src/root.zig
git commit -m "feat(web): pure WebPane state + geometry/url helpers"
```

---

## Task 2: Session `.web` kind

**Files:**
- Modify: `src/session.zig`

`src/session.zig` currently has `pub const Kind = enum { shell, viewer, editor };` and a `Session` struct whose `deinit`/`resize`/`poll` switch on `kind`. Mirror the `editor` field/branch pattern.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `src/session.zig`:

```zig
test "web session: initWeb sets kind + url, poll is inert" {
    const alloc = std.testing.allocator;
    var s = try Session.initWeb(alloc, 10, 40, "https://example.com");
    defer s.deinit();
    try std.testing.expectEqual(Kind.web, s.kind);
    try std.testing.expectEqualStrings("https://example.com", s.web.?.url());
    const r = s.poll();
    try std.testing.expect(r.alive and !r.consumed);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `.zig/zig build test 2>&1 | tail -20`
Expected: FAIL — `Kind` has no member `web` / `Session` has no `initWeb`.

- [ ] **Step 3: Implement**

At the top of `src/session.zig`, add the import after the existing `Editor` import:

```zig
const WebPane = @import("web/pane.zig").WebPane;
```

Change the `Kind` enum to:

```zig
pub const Kind = enum { shell, viewer, editor, web };
```

Add a field to the `Session` struct, after the `editor: ?Editor = null,` field:

```zig
    // Web state: pure display state; app.zig owns the WKWebView handle.
    web: ?WebPane = null,
```

Add the constructor after `initEditor`:

```zig
    /// Create a web (WKWebView) pane session. Holds only pure display state;
    /// app.zig creates and owns the native WKWebView once this pane lays out.
    pub fn initWeb(alloc: std.mem.Allocator, rows: u16, cols: u16, url: []const u8) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        const pty = Pty.initNull();
        return .{
            .term = term,
            .pty = pty,
            .kind = .web,
            .web = WebPane.init(url),
            .view_alloc = alloc,
        };
    }
```

In `deinit`, add a `.web` arm to the `switch (self.kind)` (no native cleanup here — app.zig destroys the handle on pane removal):

```zig
            .web => {},
```

In `resize`, add a `.web` arm (grid is unused for the web body; app.zig drives the NSView frame):

```zig
            .web => {},
```

In `poll`, the existing guard `if (self.kind != .shell) return .{ .alive = true, .consumed = false };` already covers `.web` — no change needed.

- [ ] **Step 4: Run the test**

Run: `.zig/zig build test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Format + commit**

```bash
.zig/zig fmt src build.zig
git add src/session.zig
git commit -m "feat(web): Session .web kind holding pure WebPane state"
```

---

## Task 3: `SessionManager.addWeb`

**Files:**
- Modify: `src/session_manager.zig`

Mirror `addEditor` (`src/session_manager.zig:261-269`).

- [ ] **Step 1: Write the failing test**

Add near the other `SessionManager` tests at the bottom of `src/session_manager.zig`:

```zig
test "addWeb creates a .web session with the given url" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    const id = try mgr.addWeb("https://example.com", 24, 80);
    const s = mgr.sessionById(id).?;
    try std.testing.expectEqual(@import("session.zig").Kind.web, s.kind);
    try std.testing.expectEqualStrings("https://example.com", s.web.?.url());
}
```

> Note: if a `sessionById` accessor does not exist, use the same lookup the
> surrounding tests use (check how `addEditor`/`addViewer` tests fetch a session;
> match that exact pattern rather than introducing a new accessor).

- [ ] **Step 2: Run it to verify it fails**

Run: `.zig/zig build test 2>&1 | tail -20`
Expected: FAIL — no `addWeb`.

- [ ] **Step 3: Implement**

Add after `addEditor` in `src/session_manager.zig`:

```zig
    /// Open `url` in a web (WKWebView) pane. The native view is created lazily
    /// by app.zig on first layout.
    pub fn addWeb(self: *SessionManager, url: []const u8, rows: u16, cols: u16) !usize {
        const id = self.next_id;
        var s = try Session.initWeb(self.alloc, rows, cols, url);
        errdefer s.deinit();
        s.id = id;
        try self.sessions.append(self.alloc, s);
        self.next_id += 1;
        return id;
    }
```

- [ ] **Step 4: Run the test**

Run: `.zig/zig build test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Format + commit**

```bash
.zig/zig fmt src build.zig
git add src/session_manager.zig
git commit -m "feat(web): SessionManager.addWeb opens a web pane"
```

---

## Task 4: Shim WKWebView lifecycle (Obj-C)

**Files:**
- Modify: `src/platform/shim.m`
- Modify: `build.zig`

This task is **native** — not unit-testable in Zig. Verify by compiling and by the `--dump` smoke check (shaders/chrome still render). The actual webview is exercised live in Task 7's manual check.

The shim already holds `static NSWindow *gWindow;` (`src/platform/shim.m:144`) and a `CAMetalLayer` (`gLayer`). Add a parallel registry of `WKWebView`s as subviews of `gWindow.contentView`.

- [ ] **Step 1: Link the WebKit framework**

In `build.zig`, after the existing `exe.root_module.linkFramework("Cocoa", .{});` block (around lines 39-46), add:

```zig
    exe.root_module.linkFramework("WebKit", .{});
```

- [ ] **Step 2: Import WebKit in the shim**

At the top of `src/platform/shim.m`, with the other `#import`s, add:

```objc
#import <WebKit/WebKit.h>
```

- [ ] **Step 3: Add the webview registry, delegate, and C functions**

Add this block to `src/platform/shim.m` (near the other helpers, before `@interface AnvilController`). It declares the Zig callback the bridge invokes, keeps a small handle→webview registry, and implements the C ABI:

```objc
// --- Web pane bridge -------------------------------------------------------
// Zig receives KVO/crash events. kind: 0=title 1=progress 2=nav 3=failed.
// For kind 0 the payload is the title UTF-8; for 1 it is the progress as a
// 4-byte float; for 2 it is two bytes (canGoBack, canGoForward); kind 3 none.
extern void anvil_web_event(void *handle, int kind, const void *payload, size_t len);

@interface AnvilWebDelegate : NSObject <WKNavigationDelegate>
@end

@implementation AnvilWebDelegate
- (void)webViewWebContentProcessDidTerminate:(WKWebView *)wv {
    // Auto-reload once; if it dies again the next call reports failed via
    // didFailProvisionalNavigation below.
    [wv reload];
}
- (void)webView:(WKWebView *)wv didFailNavigation:(WKNavigation *)n withError:(NSError *)e {
    anvil_web_event((__bridge void *)wv, 3, NULL, 0);
}
- (void)webView:(WKWebView *)wv didFailProvisionalNavigation:(WKNavigation *)n withError:(NSError *)e {
    anvil_web_event((__bridge void *)wv, 3, NULL, 0);
}
@end

static AnvilWebDelegate *gWebDelegate;

// KVO observer: forwards title/progress/canGoBack/canGoForward changes to Zig.
@interface AnvilWebObserver : NSObject
@end
@implementation AnvilWebObserver
- (void)observeValueForKeyPath:(NSString *)kp ofObject:(id)obj
                        change:(NSDictionary *)c context:(void *)ctx {
    WKWebView *wv = (WKWebView *)obj;
    void *h = (__bridge void *)wv;
    if ([kp isEqualToString:@"title"]) {
        const char *t = wv.title.UTF8String ?: "";
        anvil_web_event(h, 0, t, strlen(t));
    } else if ([kp isEqualToString:@"estimatedProgress"]) {
        float p = (float)wv.estimatedProgress;
        anvil_web_event(h, 1, &p, sizeof(p));
    } else { // canGoBack / canGoForward
        unsigned char nav[2] = { wv.canGoBack ? 1 : 0, wv.canGoForward ? 1 : 0 };
        anvil_web_event(h, 2, nav, 2);
    }
}
@end
static AnvilWebObserver *gWebObserver;

void *anvil_web_create(void) {
    if (!gWebDelegate) gWebDelegate = [[AnvilWebDelegate alloc] init];
    if (!gWebObserver) gWebObserver = [[AnvilWebObserver alloc] init];
    WKWebViewConfiguration *cfg = [[WKWebViewConfiguration alloc] init];
    // Ephemeral store: no cookies/credentials shared with native, no
    // cross-session persistence.
    cfg.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
    WKWebView *wv = [[WKWebView alloc] initWithFrame:NSZeroRect configuration:cfg];
    wv.navigationDelegate = gWebDelegate;
    wv.hidden = YES;
    [wv addObserver:gWebObserver forKeyPath:@"title" options:0 context:NULL];
    [wv addObserver:gWebObserver forKeyPath:@"estimatedProgress" options:0 context:NULL];
    [wv addObserver:gWebObserver forKeyPath:@"canGoBack" options:0 context:NULL];
    [wv addObserver:gWebObserver forKeyPath:@"canGoForward" options:0 context:NULL];
    [gWindow.contentView addSubview:wv];
    return (__bridge_retained void *)wv;
}

void anvil_web_navigate(void *handle, const char *url) {
    if (!handle) return;
    WKWebView *wv = (__bridge WKWebView *)handle;
    NSString *s = [NSString stringWithUTF8String:url];
    NSURL *u = [NSURL URLWithString:s];
    if (u) [wv loadRequest:[NSURLRequest requestWithURL:u]];
}

void anvil_web_back(void *handle)    { if (handle) [(__bridge WKWebView *)handle goBack]; }
void anvil_web_forward(void *handle) { if (handle) [(__bridge WKWebView *)handle goForward]; }
void anvil_web_reload(void *handle)  { if (handle) [(__bridge WKWebView *)handle reload]; }

// Frame is in top-left window coordinates (x,y from top); flip to AppKit's
// bottom-left origin against the content view height.
void anvil_web_set_frame(void *handle, double x, double y, double w, double h) {
    if (!handle) return;
    WKWebView *wv = (__bridge WKWebView *)handle;
    CGFloat ch = gWindow.contentView.bounds.size.height;
    wv.frame = NSMakeRect(x, ch - y - h, w, h);
}

void anvil_web_set_hidden(void *handle, bool hidden) {
    if (handle) ((__bridge WKWebView *)handle).hidden = hidden ? YES : NO;
}

void anvil_web_destroy(void *handle) {
    if (!handle) return;
    WKWebView *wv = (__bridge_transfer WKWebView *)handle;
    [wv removeObserver:gWebObserver forKeyPath:@"title"];
    [wv removeObserver:gWebObserver forKeyPath:@"estimatedProgress"];
    [wv removeObserver:gWebObserver forKeyPath:@"canGoBack"];
    [wv removeObserver:gWebObserver forKeyPath:@"canGoForward"];
    [wv removeFromSuperview];
}
```

- [ ] **Step 4: Build (the `anvil_web_event` Zig symbol does not exist yet — expect a link error, that's fine for now)**

Run: `.zig/zig build 2>&1 | tail -20`
Expected: FAIL at link with an undefined `_anvil_web_event` symbol. This is resolved in Task 5 (which adds the Zig export). Do **not** commit a non-building tree — proceed directly to Task 5 and commit them together.

> Rationale for the joined commit: the shim's `anvil_web_event` call and app.zig's
> `export fn anvil_web_event` are two halves of one ABI edge; neither builds
> without the other (the same reason the dirty-tree commit `6181a31` grouped
> shim.m with its app.zig exports). Tasks 4 and 5 share Step "commit".

---

## Task 5: app.zig native handle map + reconcile + event sink

**Files:**
- Modify: `src/app.zig`

This task is **native-boundary** — verified by `.zig/zig build` (links shim) + `exe_tests` + `--dump`. app.zig owns the `WKWebView` handle in a side map keyed by session id, creates/destroys it by diffing against live `.web` sessions, and pushes frames.

`app.zig` already computes per-pane rects in `livePanes` (see `src/app.zig:369-405`) and switches on `s.kind` for input/click/scroll. Use the existing `max_panes` and `pane.Rect` types already imported (`src/app.zig:63`).

- [ ] **Step 1: Declare the extern shim functions and the handle map**

Near the top of `src/app.zig` (with other module state like `divider_rects`, `src/app.zig:63`), add:

```zig
const webpane = @import("web/pane.zig");

extern fn anvil_web_create() callconv(.c) ?*anyopaque;
extern fn anvil_web_navigate(handle: ?*anyopaque, url: [*:0]const u8) callconv(.c) void;
extern fn anvil_web_back(handle: ?*anyopaque) callconv(.c) void;
extern fn anvil_web_forward(handle: ?*anyopaque) callconv(.c) void;
extern fn anvil_web_reload(handle: ?*anyopaque) callconv(.c) void;
extern fn anvil_web_set_frame(handle: ?*anyopaque, x: f64, y: f64, w: f64, h: f64) callconv(.c) void;
extern fn anvil_web_set_hidden(handle: ?*anyopaque, hidden: bool) callconv(.c) void;
extern fn anvil_web_destroy(handle: ?*anyopaque) callconv(.c) void;

const WebHandle = struct { id: usize, handle: ?*anyopaque };
var web_handles: [max_panes]WebHandle = undefined;
var web_handle_count: usize = 0;
```

- [ ] **Step 2: Add handle lookup + reconcile helpers**

Add these functions in `src/app.zig`:

```zig
fn webHandleFor(id: usize) ?*anyopaque {
    for (web_handles[0..web_handle_count]) |wh| {
        if (wh.id == id) return wh.handle;
    }
    return null;
}

/// Create native views for new .web sessions, destroy orphaned ones, and push
/// each live web pane's body frame + visibility. Called once per frame after
/// pane rects are known. `active_tab` panes are visible; others are hidden.
fn reconcileWebPanes(panes: []const pane.Pane) void {
    // Destroy handles whose session is no longer a live web pane.
    var i: usize = 0;
    while (i < web_handle_count) {
        const wh = web_handles[i];
        var still_live = false;
        for (panes) |p| {
            const s = mgr.sessionById(p.id) orelse continue;
            if (p.id == wh.id and s.kind == .web) { still_live = true; break; }
        }
        if (!still_live) {
            anvil_web_destroy(wh.handle);
            web_handles[i] = web_handles[web_handle_count - 1];
            web_handle_count -= 1;
        } else i += 1;
    }
    // Create + lay out live web panes.
    for (panes) |p| {
        const s = mgr.sessionById(p.id) orelse continue;
        if (s.kind != .web) continue;
        var handle = webHandleFor(p.id);
        if (handle == null and web_handle_count < max_panes) {
            handle = anvil_web_create();
            web_handles[web_handle_count] = .{ .id = p.id, .handle = handle };
            web_handle_count += 1;
            if (s.web) |*w| {
                var buf: [2048:0]u8 = undefined;
                const u = w.url();
                const n = @min(u.len, buf.len);
                @memcpy(buf[0..n], u[0..n]);
                buf[n] = 0;
                anvil_web_navigate(handle, &buf);
            }
        }
        const body = webpane.bodyFrame(.{ .x = p.rect.x, .y = p.rect.y, .w = p.rect.w, .h = p.rect.h });
        anvil_web_set_frame(handle, body.x, body.y, body.w, body.h);
        anvil_web_set_hidden(handle, false);
    }
}
```

> If `mgr.sessionById` does not exist, use the same id→session lookup app.zig
> already uses elsewhere (grep `mgr.` in app.zig for the existing accessor and
> match it). Do not invent a new one.

- [ ] **Step 3: Call reconcile from the frame path**

In `src/app.zig`'s frame/render entry (the `anvil_frame` export, which already iterates `livePanes` — see `src/app.zig:1494-1514`), after the pane rects are computed for the active tab, add a call:

```zig
    reconcileWebPanes(live);
```

(where `live` is the slice of active-tab panes already in scope; match the existing variable name used by the surrounding loop). For panes not on the active tab, ensure their handles are hidden — extend the reconcile loop's final block to set `anvil_web_set_hidden(handle, true)` for handles whose `id` is not in the active-tab `panes` slice. Implement that by, after the create+layout loop, walking `web_handles[0..web_handle_count]` and hiding any whose id is absent from `panes`.

- [ ] **Step 4: Add the event sink export**

Add to `src/app.zig`:

```zig
/// Bridge sink: WKWebView KVO/crash events. kind: 0=title 1=progress 2=nav
/// 3=failed. Updates the matching session's WebPane.
export fn anvil_web_event(handle: ?*anyopaque, kind: c_int, payload: ?*const anyopaque, len: usize) callconv(.c) void {
    var target_id: ?usize = null;
    for (web_handles[0..web_handle_count]) |wh| {
        if (wh.handle == handle) { target_id = wh.id; break; }
    }
    const id = target_id orelse return;
    const s = mgr.sessionById(id) orelse return;
    if (s.web) |*w| {
        switch (kind) {
            0 => if (payload) |p| w.setTitle(@as([*]const u8, @ptrCast(p))[0..len]),
            1 => if (payload) |p| w.setProgress(@as(*const f32, @ptrCast(@alignCast(p))).*),
            2 => if (payload != null and len >= 2) {
                const b = @as([*]const u8, @ptrCast(payload.?));
                w.setNav(b[0] != 0, b[1] != 0);
            },
            3 => w.setFailed(),
            else => {},
        }
    }
}
```

- [ ] **Step 5: Build (now links cleanly — shim + Zig export resolve each other)**

Run: `.zig/zig build 2>&1 | tail -20`
Expected: PASS (no undefined symbols).

- [ ] **Step 6: Run tests + dump smoke**

Run: `.zig/zig build test 2>&1 | tail -10 && ./zig-out/bin/anvil --dump /tmp/web_smoke.png && echo DUMP_OK`
Expected: tests PASS; `DUMP_OK` printed (Metal/chrome still render; no web pane exists in the dump path, so this only proves nothing regressed).

- [ ] **Step 7: Format + commit (Tasks 4 + 5 together)**

```bash
.zig/zig fmt src build.zig
git add build.zig src/platform/shim.m src/app.zig
git commit -m "feat(web): WKWebView shim lifecycle + app.zig handle map and event bridge"
```

---

## Task 6: Header-strip URL chrome + input/focus routing

**Files:**
- Modify: `src/app.zig`

Native-boundary. The header strip is Metal chrome (in the frame, so `--dump` covers it). Draw a 30pt band atop each web pane's rect with back/forward/reload affordances, the current URL, and a loading line. Route URL-bar editing keys.

- [ ] **Step 1: Draw the header strip**

In the same place app.zig draws other pane chrome (per-pane header strip already exists — see `chrome.header_strip_h` usage at `src/app.zig:374`), add a branch: when `s.kind == .web`, draw into the pane's top `webpane.header_strip_h` band:
- back/forward/reload glyphs (use the existing Nerd-font icon draw path; dim color when `!w.can_back` / `!w.can_fwd`),
- the URL text (`w.url()`) or, while editing, the in-progress URL-bar buffer,
- a thin progress line whose width is `w.progress * strip_w` when `w.loading`.

Use the existing chrome color tokens (`chrome.*`, mineral/alloy/mist) and the existing text/rect draw helpers app.zig already calls for the status bar — match that exact call style; do not introduce a new drawing primitive.

- [ ] **Step 2: Add URL-bar edit state + key routing**

Add module state in `src/app.zig`:

```zig
var web_urlbar_active: bool = false;
var web_urlbar_buf: [2048]u8 = undefined;
var web_urlbar_len: usize = 0;
```

In the input path (`anvil_input`, which already switches on `s.kind` — see `src/app.zig:710-728`), add a `.web` branch:
- If `web_urlbar_active`: printable bytes append to `web_urlbar_buf`; Backspace deletes; Enter validates via `webpane.validateUrl` and, if valid, calls `anvil_web_navigate(webHandleFor(id), ...)`, calls `w.beginNav(buf)`, and clears `web_urlbar_active`; Escape clears `web_urlbar_active` without navigating.
- If not active: this branch is a no-op (keystrokes reach the WKWebView directly because it is AppKit first responder; app.zig's `anvil_input` is only invoked when the Metal view is first responder).

Wire `Cmd+L` (in the existing keybind/command dispatch — grep app.zig for the keybind switch that handles `Cmd+F`/palette) to set `web_urlbar_active = true` and seed `web_urlbar_buf` from `w.url()` when the focused pane is `.web`.

- [ ] **Step 3: Wire back/forward/reload clicks**

In the click path (`anvil_mouse`, which already maps clicks to panes — see `src/app.zig:973-995`), add: if the click lands in a `.web` pane's header strip, hit-test the three affordance rects and call `anvil_web_back/forward/reload(webHandleFor(id))` accordingly. A click in the web body forwards focus to the webview (set its first responder) — call a new `anvil_web_focus(handle)` shim function (add a one-line shim fn `void anvil_web_focus(void *h){ if(h)[gWindow makeFirstResponder:(__bridge WKWebView*)h]; }` plus its `extern fn` declaration in app.zig).

- [ ] **Step 4: Build + tests + dump**

Run: `.zig/zig build 2>&1 | tail -10 && .zig/zig build test 2>&1 | tail -5 && ./zig-out/bin/anvil --dump /tmp/web_chrome.png && echo OK`
Expected: build PASS, tests PASS, `OK`. (The dump won't contain a web pane yet, but must not regress.)

- [ ] **Step 5: Format + commit**

```bash
.zig/zig fmt src build.zig
git add src/app.zig src/platform/shim.m
git commit -m "feat(web): URL-bar header strip, nav buttons, focus + key routing"
```

---

## Task 7: Invocation (palette + keybind) + live verification

**Files:**
- Modify: `src/app.zig`
- Modify: `src/palette.zig` (command list)

- [ ] **Step 1: Add an export to open a web pane**

In `src/app.zig`, mirroring `anvil_open_editor` (`src/app.zig:783`), add:

```zig
/// Open a new web pane in the active tab (splits like other new panes).
export fn anvil_open_web() callconv(.c) void {
    const rows = mgr.focusedRows();
    const cols = mgr.focusedCols();
    const id = mgr.addWeb("https://example.com", rows, cols) catch return;
    mgr.splitWith(id) catch {};
}
```

> Match the exact split/new-pane helper app.zig uses for editor/viewer panes
> (grep for how `anvil_open_editor` attaches the new session to the tab/tree —
> `addEditor` callers near `src/app.zig:752-792`). Use the same helper names;
> the `addWeb`+attach sequence must mirror the editor path, including how
> `rows/cols` are obtained.

- [ ] **Step 2: Add a command-palette entry**

In `src/palette.zig`, add a command entry "Open browser pane" to the command list (match the existing entry struct + the action enum the palette dispatches; find the entry for "Open file"/editor and copy its shape). Wire its action in app.zig's palette dispatch (the `switch` around `src/app.zig:1431` that maps `.mode_editor => anvil_set_mode(1)` etc.) to call `anvil_open_web()`.

- [ ] **Step 3: Build + tests + dump**

Run: `.zig/zig build 2>&1 | tail -10 && .zig/zig build test 2>&1 | tail -5 && ./zig-out/bin/anvil --dump /tmp/web_final.png && echo OK`
Expected: build PASS, tests PASS, `OK`.

- [ ] **Step 4: Live manual verification (cannot be automated — WKWebView is a native NSView)**

Run: `.zig/zig build run`
Then verify by hand:
1. Open the command palette, run "Open browser pane" → a split appears with a header strip and `example.com` loads in the web body.
2. `Cmd+L`, type `https://ziglang.org`, Enter → navigates; title/loading update.
3. Back/forward/reload buttons work and dim correctly at history ends.
4. Resize the window and the split divider → the web body reflows to the pane.
5. Switch to another tab and back → the web pane hides then reappears.
6. Close the pane → the web view disappears (no leak/zombie NSView).

Record the result in the commit body.

- [ ] **Step 5: Format + commit**

```bash
.zig/zig fmt src build.zig
git add src/app.zig src/palette.zig
git commit -m "feat(web): open browser pane via palette + keybind; manual verify pass"
```

---

## Task 8: Amend decision 0005 (wiki)

**Files:**
- Modify: `wiki/decisions/0005-render-host.md`
- Modify: `wiki/index.md`, `wiki/log.md`

- [ ] **Step 1: Append an amendment to 0005**

In `wiki/decisions/0005-render-host.md`, append a dated "Amendment" section recording that the content pane now exists: app chrome remains native Metal; a `WKWebView` is used **only as web-pane content** via the `anvil_web_*` shim bridge; the thin native↔web bridge (KVO events + navigate) is now realized. Reference the spec and this plan. Do **not** rewrite the original decision text.

- [ ] **Step 2: Update index + log**

Add a line to `wiki/log.md` (date, "amended 0005: WebKit content pane shipped") and ensure `wiki/index.md` reflects the amended state per its frontmatter rules.

- [ ] **Step 3: Commit**

```bash
git add wiki/decisions/0005-render-host.md wiki/index.md wiki/log.md
git commit -m "docs(wiki): amend 0005 — WebKit content pane realized"
```

---

## Self-review notes (author)

- **Spec coverage:** module boundaries (Task 1-5), compositing as NSView subview (Task 4-5), header-strip URL chrome (Task 6), thin bridge — navigate/back/forward/reload out, KVO title/progress/nav + crash in (Task 4-5), ephemeral data store + scheme block (Task 4 + `validateUrl` Task 1/6), invocation (Task 7), error handling (crash delegate + failed state, Task 4-5), testing strategy + `--dump` limit (each task), 0005 amendment (Task 8). Out-of-scope items (script-message actions, extension model, persistence, CLI verb, auto-nav) are intentionally absent.
- **Known soft spots for the implementer:** several native-task steps say "match the existing helper" rather than quoting it, because the exact app.zig accessor/split/draw helper names are not all visible in this plan's research pass. The implementer MUST grep app.zig and match the real names (`mgr.sessionById`, the split helper, the chrome draw calls) — do not invent new APIs. If a named helper (e.g. `mgr.sessionById`, `mgr.focusedRows`) does not exist, find and use the established equivalent.
- **Coordinate-flip risk:** `anvil_web_set_frame` flips top-left→bottom-left against `contentView` height. Verify visually in Task 7 step 4 #4 (resize) — if the web body is vertically mirrored, the flip is the cause.
