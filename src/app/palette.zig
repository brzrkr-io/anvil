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
    hud_toggle,
    tree_toggle,
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
    .{ .id = "hud.toggle", .title = "Toggle HUD", .subtitle = "Show or hide the developer context panel", .action = .hud_toggle },
    .{ .id = "tree.toggle", .title = "Toggle File Tree", .subtitle = "Show or hide the file explorer panel", .action = .tree_toggle },
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
