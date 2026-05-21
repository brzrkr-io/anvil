const std = @import("std");
const objc = @import("objc");
const Window = @import("app/window.zig").Window;
const Renderer = @import("render/metal.zig").Renderer;

const c = objc.c;

const win_w: usize = 1024;
const win_h: usize = 640;

/// The Caldera brand mark, embedded so the binary is self-contained.
const app_icon_png = @embedFile("assets/app-icon.png");

/// Phase-3 scratch bitmap: a synthetic gradient that proves the texture upload
/// and the Metal text pipeline. Replaced by the terminal raster in Phase 4.
var scratch: [win_w * win_h * 4]u8 = undefined;

/// Set the Dock / app-switcher icon to the Caldera brand mark.
fn setApplicationIcon(app: objc.Object) void {
    const data = objc.getClass("NSData").?.msgSend(objc.Object, "dataWithBytes:length:", .{
        app_icon_png, @as(c_ulong, app_icon_png.len),
    });
    const image = objc.getClass("NSImage").?
        .msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "initWithData:", .{data});
    app.msgSend(void, "setApplicationIconImage:", .{image});
}

/// App-delegate method: quit the process once the last window closes.
fn appShouldTerminateAfterLastWindowClosed(
    self: c.id,
    sel: c.SEL,
    sender: c.id,
) callconv(.c) bool {
    _ = self;
    _ = sel;
    _ = sender;
    return true;
}

fn fillGradient() void {
    var y: usize = 0;
    while (y < win_h) : (y += 1) {
        var x: usize = 0;
        while (x < win_w) : (x += 1) {
            const i = (y * win_w + x) * 4;
            scratch[i + 0] = 64; // B
            scratch[i + 1] = @intCast(y * 255 / win_h); // G
            scratch[i + 2] = @intCast(x * 255 / win_w); // R
            scratch[i + 3] = 255; // A
        }
    }
}

pub fn main() void {
    const NSApplication = objc.getClass("NSApplication").?;
    const app = NSApplication.msgSend(objc.Object, "sharedApplication", .{});

    // NSApplicationActivationPolicyRegular = 0 — dock icon, activatable.
    app.msgSend(void, "setActivationPolicy:", .{@as(c_long, 0)});
    setApplicationIcon(app);

    const Delegate = objc.allocateClassPair(objc.getClass("NSObject").?, "CalderaAppDelegate").?;
    _ = Delegate.addMethod(
        "applicationShouldTerminateAfterLastWindowClosed:",
        appShouldTerminateAfterLastWindowClosed,
    );
    objc.registerClassPair(Delegate);
    const delegate = Delegate.msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "init", .{});
    app.msgSend(void, "setDelegate:", .{delegate});

    const window = Window.create(
        "Caldera Console",
        @floatFromInt(win_w),
        @floatFromInt(win_h),
    );

    var renderer = Renderer.init(window.metalLayer(), win_w, win_h) catch |err| {
        std.debug.print("renderer init failed: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };

    fillGradient();
    renderer.present(&scratch);

    app.msgSend(void, "activateIgnoringOtherApps:", .{true});
    app.msgSend(void, "run", .{});
}

test {
    _ = @import("render/color.zig");
}
