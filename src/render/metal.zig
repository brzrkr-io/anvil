//! Metal renderer. The terminal is rasterized on the CPU into a BGRA8 bitmap
//! each frame; this uploads that bitmap to a texture and draws it as a single
//! full-screen quad. Shaders are Metal Shading Language compiled at runtime
//! (no offline metal toolchain needed).

const std = @import("std");
const objc = @import("objc");
const color = @import("color.zig");

const MTLClearColor = extern struct { red: f64, green: f64, blue: f64, alpha: f64 };
const MTLOrigin = extern struct { x: c_ulong, y: c_ulong, z: c_ulong };
const MTLSize = extern struct { width: c_ulong, height: c_ulong, depth: c_ulong };
const MTLRegion = extern struct { origin: MTLOrigin, size: MTLSize };

// MTLPixelFormatBGRA8Unorm
const pixel_format: c_ulong = 80;

extern fn MTLCreateSystemDefaultDevice() ?*anyopaque;

const shader_src =
    \\#include <metal_stdlib>
    \\using namespace metal;
    \\struct VOut { float4 pos [[position]]; float2 uv; };
    \\vertex VOut v_main(uint vid [[vertex_id]]) {
    \\    float2 pos[6] = {
    \\        float2(-1,-1), float2(1,-1), float2(-1,1),
    \\        float2(1,-1),  float2(1,1),  float2(-1,1)
    \\    };
    \\    float2 uvs[6] = {
    \\        float2(0,1), float2(1,1), float2(0,0),
    \\        float2(1,1), float2(1,0), float2(0,0)
    \\    };
    \\    VOut o;
    \\    o.pos = float4(pos[vid], 0, 1);
    \\    o.uv = uvs[vid];
    \\    return o;
    \\}
    \\fragment float4 f_main(VOut in [[stage_in]],
    \\                       texture2d<float> tex [[texture(0)]]) {
    \\    constexpr sampler smp(mag_filter::nearest, min_filter::nearest);
    \\    return tex.sample(smp, in.uv);
    \\}
;

fn nsString(text: [:0]const u8) objc.Object {
    return objc.getClass("NSString").?
        .msgSend(objc.Object, "stringWithUTF8String:", .{text.ptr});
}

pub const Renderer = struct {
    device: objc.Object,
    queue: objc.Object,
    layer: objc.Object,
    pipeline: objc.Object,
    texture: objc.Object,
    width: usize,
    height: usize,
    clear: MTLClearColor,

    pub fn init(layer: objc.Object, width: usize, height: usize) !Renderer {
        const device_id = MTLCreateSystemDefaultDevice() orelse return error.NoMetalDevice;
        const device = objc.Object.fromId(device_id);
        const queue = device.msgSend(objc.Object, "newCommandQueue", .{});

        layer.msgSend(void, "setDevice:", .{device});
        layer.msgSend(void, "setPixelFormat:", .{pixel_format});
        // presentsWithTransaction is toggled per-frame in present(): true only
        // during live resize (to prevent ghosting), false otherwise (lower
        // latency). Start with false; the first frame is never a live resize.
        layer.msgSend(void, "setPresentsWithTransaction:", .{false});
        layer.msgSend(void, "setDrawableSize:", .{extern struct { w: f64, h: f64 }{
            .w = @floatFromInt(width),
            .h = @floatFromInt(height),
        }});

        const pipeline = try buildPipeline(device);
        const texture = makeTexture(device, width, height);

        const cc = try color.hexToClearColor("#1a1c24"); // mineral-dark bg; setClearColor overrides per theme
        return .{
            .device = device,
            .queue = queue,
            .layer = layer,
            .pipeline = pipeline,
            .texture = texture,
            .width = width,
            .height = height,
            .clear = .{ .red = cc.r, .green = cc.g, .blue = cc.b, .alpha = cc.a },
        };
    }

    /// Update the GPU clear color. It sits behind the full-screen texture, so
    /// this only matters on resize flashes — but it must track the theme.
    pub fn setClearColor(self: *Renderer, rgb: [3]u8) void {
        self.clear = .{
            .red = @as(f64, @floatFromInt(rgb[0])) / 255.0,
            .green = @as(f64, @floatFromInt(rgb[1])) / 255.0,
            .blue = @as(f64, @floatFromInt(rgb[2])) / 255.0,
            .alpha = 1.0,
        };
    }

    /// Recreate the texture (and update the layer drawable) for a new size.
    pub fn resize(self: *Renderer, width: usize, height: usize) void {
        if (width == self.width and height == self.height) return;
        self.width = width;
        self.height = height;
        self.layer.msgSend(void, "setDrawableSize:", .{extern struct { w: f64, h: f64 }{
            .w = @floatFromInt(width),
            .h = @floatFromInt(height),
        }});
        self.texture = makeTexture(self.device, width, height);
    }

    /// Upload a `width*height*4` BGRA8 bitmap and present it as the frame.
    /// Pass `sync = true` during a live resize to prevent ghosting; for all
    /// other frames, `sync = false` uses async presentDrawable: for lower
    /// latency.
    pub fn present(self: *Renderer, pixels: []const u8, sync: bool) void {
        const pool = objc.AutoreleasePool.init();
        defer pool.deinit();

        // Toggle the layer property so the command-buffer path matches.
        self.layer.msgSend(void, "setPresentsWithTransaction:", .{sync});

        const region: MTLRegion = .{
            .origin = .{ .x = 0, .y = 0, .z = 0 },
            .size = .{ .width = self.width, .height = self.height, .depth = 1 },
        };
        self.texture.msgSend(void, "replaceRegion:mipmapLevel:withBytes:bytesPerRow:", .{
            region, @as(c_ulong, 0), pixels.ptr, @as(c_ulong, self.width * 4),
        });

        const drawable = self.layer.msgSend(objc.Object, "nextDrawable", .{});
        if (drawable.value == null) return;
        const dst = drawable.msgSend(objc.Object, "texture", .{});

        const rpd = objc.getClass("MTLRenderPassDescriptor").?
            .msgSend(objc.Object, "renderPassDescriptor", .{});
        const attachment = rpd.msgSend(objc.Object, "colorAttachments", .{})
            .msgSend(objc.Object, "objectAtIndexedSubscript:", .{@as(c_ulong, 0)});
        attachment.msgSend(void, "setTexture:", .{dst});
        attachment.msgSend(void, "setLoadAction:", .{@as(c_ulong, 2)}); // clear
        attachment.msgSend(void, "setStoreAction:", .{@as(c_ulong, 1)}); // store
        attachment.msgSend(void, "setClearColor:", .{self.clear});

        const cmd = self.queue.msgSend(objc.Object, "commandBuffer", .{});
        const enc = cmd.msgSend(objc.Object, "renderCommandEncoderWithDescriptor:", .{rpd});
        enc.msgSend(void, "setRenderPipelineState:", .{self.pipeline});
        enc.msgSend(void, "setFragmentTexture:atIndex:", .{ self.texture, @as(c_ulong, 0) });
        // MTLPrimitiveTypeTriangle = 3
        enc.msgSend(void, "drawPrimitives:vertexStart:vertexCount:", .{
            @as(c_ulong, 3), @as(c_ulong, 0), @as(c_ulong, 6),
        });
        enc.msgSend(void, "endEncoding", .{});

        if (sync) {
            // Synchronous path: commit, wait until scheduled, then present on
            // the main thread so the frame lands in lockstep with the layer's
            // resize — prevents ghosting during live resize.
            cmd.msgSend(void, "commit", .{});
            cmd.msgSend(void, "waitUntilScheduled", .{});
            drawable.msgSend(void, "present", .{});
        } else {
            // Async path: let the GPU present when ready — lower latency for
            // normal (non-resize) frames.
            cmd.msgSend(void, "presentDrawable:", .{drawable});
            cmd.msgSend(void, "commit", .{});
        }
    }
};

fn makeTexture(device: objc.Object, width: usize, height: usize) objc.Object {
    const desc = objc.getClass("MTLTextureDescriptor").?.msgSend(
        objc.Object,
        "texture2DDescriptorWithPixelFormat:width:height:mipmapped:",
        .{ pixel_format, @as(c_ulong, width), @as(c_ulong, height), false },
    );
    return device.msgSend(objc.Object, "newTextureWithDescriptor:", .{desc});
}

fn buildPipeline(device: objc.Object) !objc.Object {
    var err: objc.c.id = null;
    const library = device.msgSend(objc.Object, "newLibraryWithSource:options:error:", .{
        nsString(shader_src), @as(objc.c.id, null), &err,
    });
    if (library.value == null) {
        std.debug.print("metal: shader compile failed\n", .{});
        return error.ShaderCompileFailed;
    }

    const vfn = library.msgSend(objc.Object, "newFunctionWithName:", .{nsString("v_main")});
    const ffn = library.msgSend(objc.Object, "newFunctionWithName:", .{nsString("f_main")});

    const desc = objc.getClass("MTLRenderPipelineDescriptor").?
        .msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "init", .{});
    desc.msgSend(void, "setVertexFunction:", .{vfn});
    desc.msgSend(void, "setFragmentFunction:", .{ffn});
    desc.msgSend(objc.Object, "colorAttachments", .{})
        .msgSend(objc.Object, "objectAtIndexedSubscript:", .{@as(c_ulong, 0)})
        .msgSend(void, "setPixelFormat:", .{pixel_format});

    const pipeline = device.msgSend(objc.Object, "newRenderPipelineStateWithDescriptor:error:", .{
        desc, &err,
    });
    if (pipeline.value == null) {
        std.debug.print("metal: pipeline creation failed\n", .{});
        return error.PipelineCreationFailed;
    }
    return pipeline;
}
