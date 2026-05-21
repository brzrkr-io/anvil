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
