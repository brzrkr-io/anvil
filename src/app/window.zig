const std = @import("std");
const objc = @import("objc");

const c = objc.c;

/// Cocoa geometry structs. f64 fields, C layout — passed by value to AppKit.
pub const NSPoint = extern struct { x: f64, y: f64 };
pub const NSSize = extern struct { width: f64, height: f64 };
pub const NSRect = extern struct { origin: NSPoint, size: NSSize };

/// A native macOS window whose content view is layer-hosted by a CAMetalLayer.
pub const Window = struct {
    ns_window: objc.Object,
    content_view: objc.Object,
    layer: objc.Object,

    pub fn create(title: [:0]const u8, width: f64, height: f64) Window {
        const NSWindow = objc.getClass("NSWindow").?;
        const NSView = objc.getClass("NSView").?;
        const NSString = objc.getClass("NSString").?;
        const CAMetalLayer = objc.getClass("CAMetalLayer").?;

        const rect: NSRect = .{
            .origin = .{ .x = 0, .y = 0 },
            .size = .{ .width = width, .height = height },
        };

        // styleMask: titled(1) | closable(2) | miniaturizable(4) | resizable(8)
        const style_mask: c_ulong = 1 | 2 | 4 | 8;
        // NSBackingStoreBuffered = 2
        const backing: c_ulong = 2;

        const win = NSWindow.msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "initWithContentRect:styleMask:backing:defer:", .{
                rect, style_mask, backing, false,
            });

        const ns_title = NSString.msgSend(objc.Object, "stringWithUTF8String:", .{title.ptr});
        win.msgSend(void, "setTitle:", .{ns_title});

        const view = NSView.msgSend(objc.Object, "alloc", .{})
            .msgSend(objc.Object, "initWithFrame:", .{rect});

        // Layer-hosted view: set the layer, then opt into layer backing.
        const layer = CAMetalLayer.msgSend(objc.Object, "layer", .{});
        view.msgSend(void, "setLayer:", .{layer});
        view.msgSend(void, "setWantsLayer:", .{true});

        win.msgSend(void, "setContentView:", .{view});
        win.msgSend(void, "center", .{});
        win.msgSend(void, "makeKeyAndOrderFront:", .{@as(c.id, null)});

        return .{ .ns_window = win, .content_view = view, .layer = layer };
    }

    /// The CAMetalLayer the renderer draws into.
    pub fn metalLayer(self: Window) objc.Object {
        return self.layer;
    }
};
