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

    pub fn init(u: []const u8) WebPane {
        var w = WebPane{};
        w.setUrl(u);
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
