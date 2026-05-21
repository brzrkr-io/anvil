const std = @import("std");
const objc = @import("objc");
const Window = @import("app/window.zig").Window;
const Renderer = @import("render/metal.zig").Renderer;

const c = objc.c;

const win_w: f64 = 1024;
const win_h: f64 = 640;

/// The Caldera brand mark, embedded so the binary is self-contained.
const app_icon_png = @embedFile("assets/app-icon.png");

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

pub fn main() void {
    const NSApplication = objc.getClass("NSApplication").?;
    const app = NSApplication.msgSend(objc.Object, "sharedApplication", .{});

    // NSApplicationActivationPolicyRegular = 0 — dock icon, activatable.
    app.msgSend(void, "setActivationPolicy:", .{@as(c_long, 0)});
    setApplicationIcon(app);

    // Minimal app delegate so closing the window terminates the process.
    const Delegate = objc.allocateClassPair(objc.getClass("NSObject").?, "CalderaAppDelegate").?;
    _ = Delegate.addMethod(
        "applicationShouldTerminateAfterLastWindowClosed:",
        appShouldTerminateAfterLastWindowClosed,
    );
    objc.registerClassPair(Delegate);
    const delegate = Delegate.msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "init", .{});
    app.msgSend(void, "setDelegate:", .{delegate});

    const window = Window.create("Caldera Console", win_w, win_h);

    const renderer = Renderer.init(window.metalLayer(), win_w, win_h) catch |err| {
        std.debug.print("renderer init failed: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
    renderer.drawFrame();

    app.msgSend(void, "activateIgnoringOtherApps:", .{true});
    app.msgSend(void, "run", .{});
}

test {
    _ = @import("render/color.zig");
}
