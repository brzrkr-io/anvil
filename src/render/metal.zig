const std = @import("std");
const objc = @import("objc");
const color = @import("color.zig");

/// MTLClearColor — four normalized doubles. C layout, passed by value.
pub const MTLClearColor = extern struct { red: f64, green: f64, blue: f64, alpha: f64 };

/// CGSize — used for the layer's drawable size.
pub const CGSize = extern struct { width: f64, height: f64 };

extern fn MTLCreateSystemDefaultDevice() ?*anyopaque;

/// Clears a CAMetalLayer to a solid color. No shaders — a clear is a
/// render-pass load action.
pub const Renderer = struct {
    device: objc.Object,
    queue: objc.Object,
    layer: objc.Object,
    clear: MTLClearColor,

    pub fn init(layer: objc.Object, width: f64, height: f64) !Renderer {
        const device_id = MTLCreateSystemDefaultDevice() orelse return error.NoMetalDevice;
        const device = objc.Object.fromId(device_id);
        const queue = device.msgSend(objc.Object, "newCommandQueue", .{});

        layer.msgSend(void, "setDevice:", .{device});
        // MTLPixelFormatBGRA8Unorm = 80
        layer.msgSend(void, "setPixelFormat:", .{@as(c_ulong, 80)});
        // Fixed drawable size. On window resize, CoreAnimation scales this
        // solid-color drawable to fill the layer, so the clear survives a
        // resize untouched. A resize-driven re-render arrives in M1, once
        // there is real content whose sharpness depends on drawable size.
        layer.msgSend(void, "setDrawableSize:", .{CGSize{ .width = width, .height = height }});

        const cc = try color.hexToClearColor(color.mineral_dark_bg);
        return .{
            .device = device,
            .queue = queue,
            .layer = layer,
            .clear = .{ .red = cc.r, .green = cc.g, .blue = cc.b, .alpha = cc.a },
        };
    }

    /// Encode and present one clear pass. Safe to call before the run loop
    /// starts — the presented drawable persists until the next present.
    pub fn drawFrame(self: Renderer) void {
        const pool = objc.AutoreleasePool.init();
        defer pool.deinit();

        const drawable = self.layer.msgSend(objc.Object, "nextDrawable", .{});
        if (drawable.value == null) return; // transient — skip this frame

        const texture = drawable.msgSend(objc.Object, "texture", .{});

        const rpd = objc.getClass("MTLRenderPassDescriptor").?
            .msgSend(objc.Object, "renderPassDescriptor", .{});
        const attachment = rpd.msgSend(objc.Object, "colorAttachments", .{})
            .msgSend(objc.Object, "objectAtIndexedSubscript:", .{@as(c_ulong, 0)});
        attachment.msgSend(void, "setTexture:", .{texture});
        attachment.msgSend(void, "setLoadAction:", .{@as(c_ulong, 2)}); // MTLLoadActionClear
        attachment.msgSend(void, "setStoreAction:", .{@as(c_ulong, 1)}); // MTLStoreActionStore
        attachment.msgSend(void, "setClearColor:", .{self.clear});

        const cmd = self.queue.msgSend(objc.Object, "commandBuffer", .{});
        const encoder = cmd.msgSend(objc.Object, "renderCommandEncoderWithDescriptor:", .{rpd});
        encoder.msgSend(void, "endEncoding", .{});
        cmd.msgSend(void, "presentDrawable:", .{drawable});
        cmd.msgSend(void, "commit", .{});
    }
};
