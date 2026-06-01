# M3 — Webview Host, Typed IPC Bridge, Command Palette — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Embed a `WKWebView` in the native window, build a typed native↔web IPC bridge, and ship the first web surface — a summonable command palette.

**Architecture:** A full-window transparent `WKWebView` is layered above the Metal terminal view (Approach A), hidden by default. Pure-logic modules (`ipc/bridge.zig`, `app/palette.zig`) are TDD'd; the ObjC-bound `webview/webview.zig` is written then verified by build + manual run. `main.zig` wires it together: ⌘K summons the palette, the web side posts the chosen command back, `main.zig` runs it.

**Tech Stack:** Zig 0.16, `zig-objc`, `WKWebView` / WebKit, `std.json`, AppKit, Metal.

**Spec:** `docs/superpowers/specs/2026-05-21-m3-webview-ipc-design.md` — read it first.

---

## Notes for the implementer

- **Verified facts** (checked against the codebase and Zig 0.16 while writing this plan):
  - The project builds runtime ObjC classes with `objc.allocateClassPair(super, "Name").?` + `_ = Cls.addMethod("selector:", fn)` + `objc.registerClassPair(Cls)`. See `CalderaDelegate` / `CalderaTerminalView` in `src/main.zig`.
  - ObjC method imps have signature `fn name(_: c.id, _: c.SEL, ...args) callconv(.c) Ret` where `c = objc.c`.
  - `@embedFile` accepts a build-system anonymous-import name; `b.path` files outside `src/` reach the binary that way. This was tested empirically.
  - `std.json.parseFromSlice(T, allocator, slice, .{}) !Parsed(T)`; `Parsed(T)` has `.value` and `.deinit()`.
  - `std.json.fmt(value, .{})` returns a value formattable with `"{f}"`.
  - `type` is a valid struct field identifier in Zig 0.16; `std.json` matches a plain `type` field to the JSON key `"type"`. (`zig fmt` strips an `@"type"` escape as unnecessary — write the plain form.)
  - Zig std puts unit tests inline in the module file; `src/main.zig`'s `test { }` block imports each module so its tests run under `zig build test`.
- A change is not done until `zig build test` passes (or `zig build` for ObjC-only tasks). Report failures with output.

---

## Task 0: Branch and baseline

**Files:** none (git only)

> M3 is implemented **after the M2 config/theme branch (`feat/m2-config-theme`) is merged to `main`**. Do not start until it is merged.

- [ ] **Step 1: Confirm M2 is merged and update main**

```bash
git checkout main
git pull
git log --oneline -3
```

Expected: the M2 config/theme commits are present on `main`.

- [ ] **Step 2: Create the M3 branch**

```bash
git checkout -b feat/m3-webview-ipc
```

- [ ] **Step 3: Verify the baseline is green**

Run: `zig build test`
Expected: all tests pass, exit 0. Note the test count — later steps add to it.

---

## Task 1: IPC bridge — inbound messages and decode

**Files:**
- Create: `src/ipc/bridge.zig`
- Modify: `src/main.zig` (the `test { }` block, ~line 447)

- [ ] **Step 1: Create `src/ipc/bridge.zig` with the inbound type and tests**

```zig
//! Typed native<->web IPC message protocol for the embedded webview.
//! JSON strings cross the bridge both directions. This module owns the whole
//! message catalog: encode (native -> web) and decode (web -> native).

const std = @import("std");

// --- web -> native -------------------------------------------------------

/// A message posted by the web surface. `invoke.id` is owned by the caller's
/// allocator — call `deinit` to free it.
pub const Inbound = union(enum) {
    ready,
    invoke: []const u8,
    dismiss,

    pub fn deinit(self: Inbound, allocator: std.mem.Allocator) void {
        switch (self) {
            .invoke => |id| allocator.free(id),
            else => {},
        }
    }
};

pub const DecodeError = error{
    InvalidJson,
    UnknownType,
    MissingField,
} || std.mem.Allocator.Error;

/// Parse a JSON message from the web surface. The returned `Inbound` owns no
/// memory except `invoke.id`, which is duped into `allocator`.
pub fn decode(allocator: std.mem.Allocator, json: []const u8) DecodeError!Inbound {
    const Wire = struct {
        @"type": []const u8,
        id: ?[]const u8 = null,
    };
    const parsed = std.json.parseFromSlice(Wire, allocator, json, .{
        .ignore_unknown_fields = true,
    }) catch return error.InvalidJson;
    defer parsed.deinit();

    const w = parsed.value;
    if (std.mem.eql(u8, w.@"type", "ready")) return .ready;
    if (std.mem.eql(u8, w.@"type", "dismiss")) return .dismiss;
    if (std.mem.eql(u8, w.@"type", "invoke")) {
        const id = w.id orelse return error.MissingField;
        return .{ .invoke = try allocator.dupe(u8, id) };
    }
    return error.UnknownType;
}

test "decode ready" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"ready\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .ready);
}

test "decode dismiss" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"dismiss\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .dismiss);
}

test "decode invoke carries the command id" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"invoke\",\"id\":\"theme.dark\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .invoke);
    try std.testing.expectEqualStrings("theme.dark", msg.invoke);
}

test "decode ignores unknown fields" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"ready\",\"extra\":99}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .ready);
}

test "decode invoke without id fails" {
    try std.testing.expectError(error.MissingField, decode(std.testing.allocator, "{\"type\":\"invoke\"}"));
}

test "decode unknown type fails" {
    try std.testing.expectError(error.UnknownType, decode(std.testing.allocator, "{\"type\":\"banana\"}"));
}

test "decode malformed json fails" {
    try std.testing.expectError(error.InvalidJson, decode(std.testing.allocator, "{not json"));
}
```

- [ ] **Step 2: Register the module in `main.zig`'s test block**

In `src/main.zig`, find the `test { }` block at the end of the file and add this line inside it, after the last existing `_ = @import(...)`:

```zig
    _ = @import("ipc/bridge.zig");
```

- [ ] **Step 3: Run the tests**

Run: `zig build test`
Expected: PASS — the 7 new `decode` tests pass alongside the existing suite.

- [ ] **Step 4: Commit**

```bash
git add src/ipc/bridge.zig src/main.zig
git commit -m "feat(ipc): typed bridge — inbound message decode"
```

---

## Task 2: IPC bridge — outbound messages and encode

**Files:**
- Modify: `src/ipc/bridge.zig`

- [ ] **Step 1: Add the outbound types and `encode` to `src/ipc/bridge.zig`**

Insert this block after the `decode` function and before the first `test`:

```zig
// --- native -> web -------------------------------------------------------

/// One selectable command shown in the palette.
pub const Command = struct {
    id: []const u8,
    title: []const u8,
    subtitle: ?[]const u8 = null,
};

/// The theme colors the web surface needs to match the terminal. Hex strings.
pub const ThemeTokens = struct {
    background: []const u8,
    foreground: []const u8,
    accent: []const u8,
};

/// A message sent to the web surface.
pub const Outbound = union(enum) {
    show: struct {
        commands: []const Command,
        theme: ThemeTokens,
    },
    hide,
};

/// Serialize an outbound message to a JSON string owned by `allocator`.
pub fn encode(allocator: std.mem.Allocator, msg: Outbound) std.mem.Allocator.Error![]u8 {
    switch (msg) {
        .hide => return allocator.dupe(u8, "{\"type\":\"hide\"}"),
        .show => |s| {
            const Wire = struct {
                @"type": []const u8 = "show",
                commands: []const Command,
                theme: ThemeTokens,
            };
            return std.fmt.allocPrint(allocator, "{f}", .{
                std.json.fmt(Wire{ .commands = s.commands, .theme = s.theme }, .{}),
            });
        },
    }
}
```

- [ ] **Step 2: Add encode tests at the end of `src/ipc/bridge.zig`**

```zig
test "encode hide" {
    const json = try encode(std.testing.allocator, .hide);
    defer std.testing.allocator.free(json);
    try std.testing.expectEqualStrings("{\"type\":\"hide\"}", json);
}

test "encode show" {
    const cmds = [_]Command{.{ .id = "x", .title = "X" }};
    const json = try encode(std.testing.allocator, .{ .show = .{
        .commands = &cmds,
        .theme = .{ .background = "#000000", .foreground = "#ffffff", .accent = "#2f7f86" },
    } });
    defer std.testing.allocator.free(json);
    try std.testing.expectEqualStrings(
        "{\"type\":\"show\",\"commands\":[{\"id\":\"x\",\"title\":\"X\",\"subtitle\":null}]," ++
            "\"theme\":{\"background\":\"#000000\",\"foreground\":\"#ffffff\",\"accent\":\"#2f7f86\"}}",
        json,
    );
}

test "encode show emits a subtitle when present" {
    const cmds = [_]Command{.{ .id = "x", .title = "X", .subtitle = "hint" }};
    const json = try encode(std.testing.allocator, .{ .show = .{
        .commands = &cmds,
        .theme = .{ .background = "#000000", .foreground = "#ffffff", .accent = "#2f7f86" },
    } });
    defer std.testing.allocator.free(json);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"subtitle\":\"hint\"") != null);
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test`
Expected: PASS — the 3 new `encode` tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/ipc/bridge.zig
git commit -m "feat(ipc): typed bridge — outbound message encode"
```

---

## Task 3: Command palette controller

**Files:**
- Create: `src/app/palette.zig`
- Modify: `src/main.zig` (the `test { }` block)

- [ ] **Step 1: Create `src/app/palette.zig` with the catalog, the controller, and tests**

```zig
//! Command-palette controller: the static command catalog, the id->action
//! mapping, and the summon/dismiss + ready-handshake state machine. No ObjC.

const std = @import("std");

/// A native action a palette command runs. The host maps each to real work.
pub const Action = enum {
    theme_dark,
    theme_light,
    config_reload,
    clear_screen,
    scroll_top,
    scroll_bottom,
    app_quit,
};

pub const Entry = struct {
    id: []const u8,
    title: []const u8,
    subtitle: ?[]const u8 = null,
    action: Action,
};

/// The M3 command set. Every later feature registers into this catalog.
pub const catalog = [_]Entry{
    .{ .id = "theme.dark", .title = "Switch to Dark Theme", .action = .theme_dark },
    .{ .id = "theme.light", .title = "Switch to Light Theme", .action = .theme_light },
    .{ .id = "config.reload", .title = "Reload Config", .action = .config_reload },
    .{ .id = "terminal.clear", .title = "Clear Screen", .action = .clear_screen },
    .{ .id = "scroll.top", .title = "Scroll to Top", .action = .scroll_top },
    .{ .id = "scroll.bottom", .title = "Scroll to Bottom", .action = .scroll_bottom },
    .{ .id = "app.quit", .title = "Quit Anvil", .action = .app_quit },
};

/// Look up the action for a command id, or null if unknown.
pub fn actionForId(id: []const u8) ?Action {
    for (catalog) |e| {
        if (std.mem.eql(u8, e.id, id)) return e.action;
    }
    return null;
}

/// Tracks palette visibility and the webview ready-handshake. A summon before
/// the webview signals `ready` is deferred and flushed on `onReady`.
pub const Palette = struct {
    visible: bool = false,
    ready: bool = false,
    pending_show: bool = false,

    /// Mark the palette summoned. Returns true if the host should send the
    /// `show` message now; false if it must wait for `onReady`.
    pub fn summon(self: *Palette) bool {
        self.visible = true;
        if (self.ready) return true;
        self.pending_show = true;
        return false;
    }

    /// The webview finished loading. Returns true if a deferred `show` should
    /// be sent now.
    pub fn onReady(self: *Palette) bool {
        self.ready = true;
        if (self.pending_show) {
            self.pending_show = false;
            return true;
        }
        return false;
    }

    /// Mark the palette dismissed.
    pub fn dismiss(self: *Palette) void {
        self.visible = false;
        self.pending_show = false;
    }
};

test "catalog ids map to actions" {
    try std.testing.expectEqual(Action.theme_dark, actionForId("theme.dark").?);
    try std.testing.expectEqual(Action.app_quit, actionForId("app.quit").?);
    try std.testing.expectEqual(Action.scroll_top, actionForId("scroll.top").?);
}

test "unknown id has no action" {
    try std.testing.expect(actionForId("nope.nope") == null);
}

test "summon after ready shows immediately" {
    var p = Palette{ .ready = true };
    try std.testing.expect(p.summon());
    try std.testing.expect(p.visible);
    try std.testing.expect(!p.pending_show);
}

test "summon before ready defers the show" {
    var p = Palette{};
    try std.testing.expect(!p.summon());
    try std.testing.expect(p.visible);
    try std.testing.expect(p.pending_show);
}

test "onReady flushes a deferred show exactly once" {
    var p = Palette{};
    _ = p.summon();
    try std.testing.expect(p.onReady());
    try std.testing.expect(p.ready);
    try std.testing.expect(!p.pending_show);
}

test "onReady with no pending show returns false" {
    var p = Palette{};
    try std.testing.expect(!p.onReady());
}

test "dismiss clears visibility and pending state" {
    var p = Palette{};
    _ = p.summon();
    p.dismiss();
    try std.testing.expect(!p.visible);
    try std.testing.expect(!p.pending_show);
}
```

- [ ] **Step 2: Register the module in `main.zig`'s test block**

Add to the `test { }` block in `src/main.zig`:

```zig
    _ = @import("app/palette.zig");
```

- [ ] **Step 3: Run the tests**

Run: `zig build test`
Expected: PASS — the 7 new palette tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/app/palette.zig src/main.zig
git commit -m "feat(palette): command catalog and summon/dismiss controller"
```

---

## Task 4: Command palette web surface

**Files:**
- Create: `ui/palette/index.html`

- [ ] **Step 1: Create `ui/palette/index.html`**

```html
<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  :root { --bg: #0b0d0e; --fg: #e8e6e3; --accent: #2f7f86; }
  html, body { margin: 0; height: 100%; }
  body {
    background: transparent;
    display: none;
    font-family: 'IBM Plex Sans', -apple-system, sans-serif;
  }
  body.visible { display: block; }
  #backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex; justify-content: center; align-items: flex-start;
  }
  #palette {
    margin-top: 12vh; width: 540px; max-width: 90vw;
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 8px; overflow: hidden;
    box-shadow: 0 12px 48px rgba(0, 0, 0, 0.5);
  }
  #query {
    width: 100%; box-sizing: border-box; padding: 14px 16px;
    background: var(--bg); color: var(--fg);
    border: none; outline: none; font-size: 15px;
    font-family: 'IBM Plex Mono', ui-monospace, monospace;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  #list { list-style: none; margin: 0; padding: 6px; max-height: 320px; overflow-y: auto; }
  #list li {
    padding: 9px 12px; border-radius: 5px; color: var(--fg);
    cursor: pointer; display: flex; justify-content: space-between;
  }
  #list li .sub { opacity: 0.5; font-size: 12px; }
  #list li.sel { background: var(--accent); color: var(--bg); }
  #list li.sel .sub { opacity: 0.8; }
</style>
</head>
<body>
<div id="backdrop">
  <div id="palette">
    <input id="query" type="text" placeholder="Type a command…" autocomplete="off" spellcheck="false">
    <ul id="list"></ul>
  </div>
</div>
<script>
(function () {
  var commands = [], filtered = [], sel = 0;
  var body = document.body;
  var query = document.getElementById('query');
  var list = document.getElementById('list');
  var backdrop = document.getElementById('backdrop');

  function post(obj) {
    window.webkit.messageHandlers.caldera.postMessage(JSON.stringify(obj));
  }

  function fuzzy(needle, hay) {
    needle = needle.toLowerCase();
    hay = hay.toLowerCase();
    var i = 0;
    for (var j = 0; j < hay.length && i < needle.length; j++) {
      if (hay[j] === needle[i]) i++;
    }
    return i === needle.length;
  }

  function render() {
    var q = query.value.trim();
    filtered = commands.filter(function (c) { return q === '' || fuzzy(q, c.title); });
    if (sel >= filtered.length) sel = Math.max(0, filtered.length - 1);
    list.innerHTML = '';
    filtered.forEach(function (c, idx) {
      var li = document.createElement('li');
      if (idx === sel) li.className = 'sel';
      var title = document.createElement('span');
      title.textContent = c.title;
      li.appendChild(title);
      if (c.subtitle) {
        var sub = document.createElement('span');
        sub.className = 'sub';
        sub.textContent = c.subtitle;
        li.appendChild(sub);
      }
      li.addEventListener('click', function () { invoke(idx); });
      list.appendChild(li);
    });
  }

  function invoke(idx) {
    if (idx >= 0 && idx < filtered.length) {
      post({ type: 'invoke', id: filtered[idx].id });
    }
  }

  window.caldera = {
    receive: function (msg) {
      if (msg.type === 'show') {
        commands = msg.commands || [];
        if (msg.theme) {
          var r = document.documentElement.style;
          r.setProperty('--bg', msg.theme.background);
          r.setProperty('--fg', msg.theme.foreground);
          r.setProperty('--accent', msg.theme.accent);
        }
        query.value = '';
        sel = 0;
        render();
        body.classList.add('visible');
        query.focus();
      } else if (msg.type === 'hide') {
        body.classList.remove('visible');
        query.value = '';
        sel = 0;
      }
    }
  };

  query.addEventListener('input', render);

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      post({ type: 'dismiss' });
      e.preventDefault();
    } else if (e.key === 'ArrowDown') {
      if (sel < filtered.length - 1) { sel++; render(); }
      e.preventDefault();
    } else if (e.key === 'ArrowUp') {
      if (sel > 0) { sel--; render(); }
      e.preventDefault();
    } else if (e.key === 'Enter') {
      invoke(sel);
      e.preventDefault();
    }
  });

  backdrop.addEventListener('mousedown', function (e) {
    if (e.target === backdrop) post({ type: 'dismiss' });
  });

  post({ type: 'ready' });
})();
</script>
</body>
</html>
```

- [ ] **Step 2: Commit**

```bash
git add ui/palette/index.html
git commit -m "feat(ui): command palette web surface"
```

---

## Task 5: Build wiring — embed the web surface, link WebKit

**Files:**
- Modify: `build.zig`

- [ ] **Step 1: Embed the web asset and link the WebKit framework**

In `build.zig`, after the `exe_mod` framework links (the `linkFramework` lines, ~line 21-26), add the WebKit link and the anonymous import:

```zig
    exe_mod.linkFramework("AppKit", .{});
    exe_mod.linkFramework("Metal", .{});
    exe_mod.linkFramework("QuartzCore", .{});
    exe_mod.linkFramework("CoreText", .{});
    exe_mod.linkFramework("CoreGraphics", .{});
    exe_mod.linkFramework("CoreFoundation", .{});
    exe_mod.linkFramework("WebKit", .{});

    exe_mod.addAnonymousImport("palette_html", .{
        .root_source_file = b.path("ui/palette/index.html"),
    });
```

- [ ] **Step 2: Verify the build still compiles**

Run: `zig build`
Expected: exit 0, no warnings. (`palette_html` is not yet referenced — that is fine; an unreferenced import is not an error.)

- [ ] **Step 3: Commit**

```bash
git add build.zig
git commit -m "build: link WebKit and embed the palette web surface"
```

---

## Task 6: Webview host

**Files:**
- Create: `src/webview/webview.zig`

> This module is ObjC-bound. It is not unit-tested (consistent with `render/metal.zig` and `pty/pty.zig`); it is verified by compilation here and by manual run in Task 8.

- [ ] **Step 1: Create `src/webview/webview.zig`**

```zig
//! WKWebView host: embeds one transparent webview above the Metal terminal
//! view, registers the web->native script-message handler, and exposes
//! show / hide / setFrame / evalJS. ObjC-bound; no unit tests.

const std = @import("std");
const objc = @import("objc");
const c = objc.c;

const CGPoint = extern struct { x: f64, y: f64 };
const CGSize = extern struct { width: f64, height: f64 };
const CGRect = extern struct { origin: CGPoint, size: CGSize };

/// Set by the host before the webview can post anything. Receives the raw
/// JSON string the web surface posts via `postMessage`.
pub var on_message: ?*const fn (json: []const u8) void = null;

fn nsString(text: [:0]const u8) objc.Object {
    return objc.getClass("NSString").?
        .msgSend(objc.Object, "stringWithUTF8String:", .{text.ptr});
}

/// ObjC imp for `userContentController:didReceiveScriptMessage:`. The web
/// surface always posts a JSON string, so `message.body` is an NSString.
fn imScriptMessage(_: c.id, _: c.SEL, _: c.id, message: c.id) callconv(.c) void {
    const msg = objc.Object.fromId(message);
    const body = msg.msgSend(objc.Object, "body", .{});
    if (body.value == null) return;
    const cstr = body.msgSend(?[*:0]const u8, "UTF8String", .{}) orelse return;
    if (on_message) |cb| cb(std.mem.span(cstr));
}

pub const Webview = struct {
    obj: objc.Object,
    window: objc.Object,

    /// Create the webview, add it as a hidden subview of `container`, and load
    /// `html`. `width`/`height` are the initial frame size in points.
    pub fn init(
        window: objc.Object,
        container: objc.Object,
        width: f64,
        height: f64,
        html: [:0]const u8,
    ) Webview {
        const Handler = objc.allocateClassPair(
            objc.getClass("NSObject").?,
            "CalderaScriptHandler",
        ).?;
        _ = Handler.addMethod("userContentController:didReceiveScriptMessage:", imScriptMessage);
        objc.registerClassPair(Handler);
        const handler = Handler.msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "init", .{});

        const ucc = objc.getClass("WKUserContentController").?
            .msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "init", .{});
        ucc.msgSend(void, "addScriptMessageHandler:name:", .{ handler, nsString("caldera") });

        const config = objc.getClass("WKWebViewConfiguration").?
            .msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "init", .{});
        config.msgSend(void, "setUserContentController:", .{ucc});

        const frame: CGRect = .{ .origin = .{ .x = 0, .y = 0 }, .size = .{ .width = width, .height = height } };
        const webview = objc.getClass("WKWebView").?
            .msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "initWithFrame:configuration:", .{ frame, config });

        // Transparent: the web-drawn dim backdrop shows the terminal through.
        const no = objc.getClass("NSNumber").?
            .msgSend(objc.Object, "numberWithBool:", .{false});
        webview.msgSend(void, "setValue:forKey:", .{ no, nsString("drawsBackground") });
        webview.msgSend(void, "setHidden:", .{true});

        container.msgSend(void, "addSubview:", .{webview});
        webview.msgSend(void, "loadHTMLString:baseURL:", .{ nsString(html), @as(c.id, null) });

        return .{ .obj = webview, .window = window };
    }

    /// Make the webview visible and give it keyboard focus.
    pub fn show(self: Webview) void {
        self.obj.msgSend(void, "setHidden:", .{false});
        self.window.msgSend(void, "makeFirstResponder:", .{self.obj});
    }

    /// Hide the webview and return keyboard focus to `terminal_view`.
    pub fn hide(self: Webview, terminal_view: objc.Object) void {
        self.obj.msgSend(void, "setHidden:", .{true});
        self.window.msgSend(void, "makeFirstResponder:", .{terminal_view});
    }

    /// Resize the webview frame to fill the window content area (points).
    pub fn setFrame(self: Webview, width: f64, height: f64) void {
        const frame: CGRect = .{ .origin = .{ .x = 0, .y = 0 }, .size = .{ .width = width, .height = height } };
        self.obj.msgSend(void, "setFrame:", .{frame});
    }

    /// Run JavaScript in the webview.
    pub fn evalJS(self: Webview, js: [:0]const u8) void {
        self.obj.msgSend(void, "evaluateJavaScript:completionHandler:", .{ nsString(js), @as(c.id, null) });
    }
};
```

- [ ] **Step 2: Verify it compiles**

Run: `zig build`
Expected: exit 0. (`webview.zig` is not imported anywhere yet — Task 7 wires it in. This step only checks the file itself has no syntax errors by building the project; if the unreferenced file is not compiled, the real check is Step 2 of Task 7. Proceed either way.)

- [ ] **Step 3: Commit**

```bash
git add src/webview/webview.zig
git commit -m "feat(webview): WKWebView host with script-message handler"
```

---

## Task 7: Wire the webview, bridge, and palette into the app

**Files:**
- Modify: `src/main.zig`

- [ ] **Step 1: Add the module imports**

In `src/main.zig`, after the existing import block (~line 17, after `const keys = ...`), add:

```zig
const webview_mod = @import("webview/webview.zig");
const palette_mod = @import("app/palette.zig");
const bridge = @import("ipc/bridge.zig");
```

- [ ] **Step 2: Embed the web surface**

After the `const app_icon_png = @embedFile("assets/app-icon.png");` line (~line 23), add:

```zig
const palette_html: [:0]const u8 = @embedFile("palette_html");
```

- [ ] **Step 3: Add webview and palette fields to `App`**

In the `App` struct, add two fields as the last fields, right after `search_open`:

```zig
    search: Search,
    search_open: bool = false,
    webview: webview_mod.Webview,
    palette: palette_mod.Palette = .{},
};
```

- [ ] **Step 4: Add the palette wiring functions**

Add this block to `src/main.zig` after the `loadKeybindings` function (after its closing brace, ~line 122):

```zig
// --- command palette -----------------------------------------------------

fn formatHex(buf: *[8]u8, rgb: [3]u8) []const u8 {
    return std.fmt.bufPrint(buf, "#{x:0>2}{x:0>2}{x:0>2}", .{ rgb[0], rgb[1], rgb[2] }) catch "#000000";
}

fn sendShow() void {
    var cmds: [palette_mod.catalog.len]bridge.Command = undefined;
    for (palette_mod.catalog, 0..) |e, i| {
        cmds[i] = .{ .id = e.id, .title = e.title, .subtitle = e.subtitle };
    }
    var bg: [8]u8 = undefined;
    var fg: [8]u8 = undefined;
    var ac: [8]u8 = undefined;
    const json = bridge.encode(g.alloc, .{ .show = .{
        .commands = &cmds,
        .theme = .{
            .background = formatHex(&bg, g.theme.background),
            .foreground = formatHex(&fg, g.theme.foreground),
            .accent = formatHex(&ac, g.theme.accent),
        },
    } }) catch return;
    defer g.alloc.free(json);
    const js = std.fmt.allocPrintZ(g.alloc, "window.caldera.receive({s});", .{json}) catch return;
    defer g.alloc.free(js);
    g.webview.evalJS(js);
}

fn summonPalette() void {
    if (g.palette.summon()) {
        sendShow();
        g.webview.show();
    }
    // Not ready yet: handleReady() will send `show` and reveal the webview.
}

fn handleReady() void {
    if (g.palette.onReady()) {
        sendShow();
        g.webview.show();
    }
}

fn hidePalette() void {
    g.palette.dismiss();
    const json = bridge.encode(g.alloc, .hide) catch return;
    defer g.alloc.free(json);
    const js = std.fmt.allocPrintZ(g.alloc, "window.caldera.receive({s});", .{json}) catch return;
    defer g.alloc.free(js);
    g.webview.evalJS(js);
    g.webview.hide(g.view);
}

fn setTheme(name: []const u8) void {
    g.theme = theme_mod.byName(name);
    g.renderer.setClearColor(g.theme.background);
    g.dirty = true;
}

fn runAction(action: palette_mod.Action) void {
    switch (action) {
        .theme_dark => setTheme("mineral-dark"),
        .theme_light => setTheme("mineral-light"),
        .config_reload => {
            if (g.watcher.path.len > 0) {
                applyConfig(cfg_mod.load(g.alloc, g.watcher.path));
            }
        },
        .clear_screen => {
            g.tabs.current().terminal.feed("\x1b[H\x1b[2J");
            g.dirty = true;
        },
        .scroll_top => {
            const t = &g.tabs.current().terminal;
            t.scrollViewport(@intCast(t.scrollbackLen()));
            g.dirty = true;
        },
        .scroll_bottom => {
            g.tabs.current().terminal.scrollToBottom();
            g.dirty = true;
        },
        .app_quit => g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)}),
    }
}

fn handleWebMessage(json: []const u8) void {
    const msg = bridge.decode(g.alloc, json) catch |e| {
        std.debug.print("anvil: webview message decode failed: {s}\n", .{@errorName(e)});
        return;
    };
    defer msg.deinit(g.alloc);
    switch (msg) {
        .ready => handleReady(),
        .dismiss => hidePalette(),
        .invoke => |id| {
            if (palette_mod.actionForId(id)) |action| {
                hidePalette();
                runAction(action);
            } else {
                std.debug.print("anvil: unknown command id: {s}\n", .{id});
            }
        },
    }
}
```

- [ ] **Step 5: Intercept ⌘K inside `onKeyDown`'s ⌘ branch**

`onKeyDown` already has an `if (mods.command) { ... }` branch that routes ⌘-combos through `handleTabKey`. **Do not replace `onKeyDown`** — that would destroy tab shortcuts (`handleTabKey`) and the search-bar editing path (`g.search_open`). Instead, add the ⌘K check inside that branch, right after the `handleTabKey` check. Find this exact block in `onKeyDown`:

```zig
    if (mods.command) {
        const src = event.msgSend(objc.Object, "charactersIgnoringModifiers", .{});
        if (firstCodepoint(src)) |cp| {
            if (handleTabKey(mods, cp)) return;
        }
        return; // other ⌘ combos still go to the system
    }
```

and replace it with:

```zig
    if (mods.command) {
        const src = event.msgSend(objc.Object, "charactersIgnoringModifiers", .{});
        if (firstCodepoint(src)) |cp| {
            if (handleTabKey(mods, cp)) return;
            // ⌘K — summon the command palette.
            if (asciiLowerCp(cp) == 'k' and !mods.shift and !mods.control and !mods.option) {
                summonPalette();
                return;
            }
        }
        return; // other ⌘ combos still go to the system
    }
```

`asciiLowerCp` and `firstCodepoint` are existing file-scope helpers in `main.zig` — no new helper needed.

- [ ] **Step 6: Resize the webview on window resize**

The merged `onResize` is two lines (`resizeAllTabs(); renderFrame();`). Replace the whole function with:

```zig
fn onResize() void {
    resizeAllTabs();
    const b = g.view.msgSend(CGRect, "bounds", .{});
    g.webview.setFrame(b.size.width, b.size.height);
    renderFrame();
}
```

- [ ] **Step 7: Create the webview in `main()` and register the message callback**

In `main()`, immediately before the `g = .{` struct literal, add this line (the `window`, `view`, and `cfg` locals all exist by that point):

```zig
    const wv = webview_mod.Webview.init(window, view, cfg.window.width, cfg.window.height, palette_html);
```

Then add the `webview` field to the `g = .{ ... }` literal. The literal's last field is `.search = Search.init(alloc),` — add `.webview` after it:

```zig
        .search = Search.init(alloc),
        .webview = wv,
    };
```

(`.palette` is not set in the literal — it has a default `.{}` in the `App` struct.)

Then, immediately after the `loadKeybindings(cfg.keybindings);` line (which follows `g.renderer.setClearColor(active_theme.background);`), add:

```zig
    webview_mod.on_message = handleWebMessage;
```

- [ ] **Step 8: Build and run the unit suite**

Run: `zig build test`
Expected: PASS — 210 tests (193 baseline + 17 from Tasks 1-3). Task 7 adds no tests but must not break the build or the suite.

- [ ] **Step 9: Commit**

```bash
git add src/main.zig
git commit -m "feat(app): summon the command palette with Cmd-K and run its commands"
```

---

## Task 8: End-to-end verification and wiki update

**Files:**
- Modify: `wiki/index.md`, `wiki/log.md`, `todo.txt`

- [ ] **Step 1: Build and run the app**

Run: `zig build run`
Expected: the terminal window opens and zsh renders as before.

- [ ] **Step 2: Manual verification checklist**

Confirm each, in order:

1. Press ⌘K — the command palette appears centered over the terminal, with a dimmed backdrop; the terminal is still visible (and still rendering) underneath.
2. Type `dark` then `light` — the list fuzzy-filters to the matching theme command.
3. With "Switch to Light Theme" selected, press Enter — the palette closes and the terminal **actually switches to the light theme**.
4. Press ⌘K again, choose "Switch to Dark Theme" — the terminal returns to dark.
5. Press ⌘K, then Esc — the palette dismisses and keyboard focus returns to the terminal (typing goes to the shell again).
6. Press ⌘K, choose "Clear Screen" — the visible screen clears.
7. Press ⌘K, choose "Scroll to Top" / "Scroll to Bottom" — the viewport jumps.
8. Resize the window with the palette open — the overlay stays aligned to the window.
9. Press ⌘K, choose "Quit Anvil" — the app exits.

If any step fails, fix it before continuing — do not mark this task complete with a failing checklist item.

- [ ] **Step 3: Update `wiki/index.md`**

In the "Current State" section, change the status line to note M3:

```
- Status: M0–M3 complete — the hybrid app: a GPU terminal plus an embedded
  webview with a typed IPC bridge and a command-palette overlay (⌘K). M2's
  multi-tab, search, and shell-integration sub-projects remain.
```

- [ ] **Step 4: Append `wiki/log.md`**

Add a dated entry:

```
- 2026-05-21 — M3 complete: webview host, typed IPC bridge, command palette.
  New `src/ipc/bridge.zig` (typed Inbound/Outbound JSON messages),
  `src/app/palette.zig` (command catalog + summon/dismiss controller),
  `src/webview/webview.zig` (WKWebView host above the Metal view, transparent,
  hidden by default), `ui/palette/index.html` (the web surface, embedded via a
  build anonymous import). `main.zig` summons the palette on ⌘K and runs the
  chosen command. WebKit framework linked. 1XX tests pass. Spec:
  docs/superpowers/specs/2026-05-21-m3-webview-ipc-design.md.
```

Replace `1XX` with the actual `zig build test` count.

- [ ] **Step 5: Update `todo.txt`**

Mark M3 done in the ROADMAP section and move it from the roadmap to DONE, mirroring how M0/M1 are recorded.

- [ ] **Step 6: Final verification**

Run: `zig build test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add wiki/index.md wiki/log.md todo.txt
git commit -m "docs: record M3 webview/IPC/palette completion"
```

---

## Self-review (completed while writing this plan)

- **Spec coverage:** webview host → Task 6; typed IPC bridge → Tasks 1-2; command palette controller → Task 3; web surface → Task 4; embed/build → Task 5; ⌘K summon, dismiss, message routing, resize, ready handshake → Task 7; the five coexistence sharp edges from the spec → Task 6 (`setHidden:true` default, transparency) + Task 7 (first-responder on summon/dismiss, resize) + Task 3/7 (ready handshake gates `show`); error handling (log-and-drop) → Task 7 `handleWebMessage`; testing → Tasks 1-3 unit tests + Task 8 manual checklist. The spec's `Outbound.hide` is used (Task 7 `hidePalette` sends it; the web surface resets its state on it). No gaps.
- **Type consistency:** `Inbound` / `Outbound` / `Command` / `ThemeTokens` / `decode` / `encode` (bridge.zig) are used with matching shapes in Task 7. `Action` / `catalog` / `actionForId` / `Palette` (palette.zig) match Task 7 usage. `Webview.init` takes `(window, container, width, height, html)` in Task 6 and is called that way in Task 7. `on_message` signature matches `handleWebMessage`.
- **Placeholders:** none — every code step is complete. (`1XX` in Task 8 Step 4 is an instruction to substitute the real count, not a code placeholder.)
