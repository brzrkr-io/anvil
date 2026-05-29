const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const Parser = @import("vt/parser.zig").Parser;
const Pty = @import("pty.zig").Pty;
const Renderer = @import("render/renderer.zig").Renderer;
const inst = @import("render/instance.zig");

const shader_src = @embedFile("platform/shaders.metal");
const max_instances = 60000;
const font_pt: f32 = 13.0;

var term: Terminal = undefined;
var parser: Parser = .{};
var pty: Pty = undefined;
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = 8 };
var instances: [max_instances]inst.CellInstance = undefined;
var ready = false;
var spawned = false;

const AtlasParams = extern struct {
    first: u32,
    count: u32,
    cols: u32,
    rows: u32,
    pt_size: f32,
};

export fn anvil_shader_src(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = shader_src.len;
    return shader_src.ptr;
}

export fn anvil_atlas_params(out: *AtlasParams) callconv(.c) void {
    const a = renderer.atlas;
    out.* = .{ .first = a.first, .count = a.count, .cols = a.cols, .rows = a.rows(), .pt_size = font_pt };
}

export fn anvil_set_metrics(cell_w: f32, cell_h: f32) callconv(.c) void {
    renderer.cell_w = cell_w;
    renderer.cell_h = cell_h;
}

export fn anvil_resize(px_w: f32, px_h: f32) callconv(.c) void {
    const g = renderer.gridSize(px_w, px_h);
    if (ready and g.cols == term.grid.cols and g.rows == term.grid.rows) return;
    if (ready) term.deinit();
    term = Terminal.init(std.heap.page_allocator, g.rows, g.cols) catch return;
    ready = true;

    if (!spawned) {
        pty = Pty.spawn(g.rows, g.cols) catch return;
        pty.setNonblock();
        spawned = true;
    } else {
        pty.resize(g.rows, g.cols);
    }
}

/// Drain pending shell output into the terminal. Returns 0 when the shell has
/// exited (EOF) so the front-end can quit.
export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    var buf: [8192]u8 = undefined;
    while (true) {
        switch (pty.read(&buf)) {
            .data => |n| parser.feed(&term, buf[0..n]),
            .would_block => return 1,
            .eof => return 0,
        }
    }
}

export fn anvil_input(ptr: [*]const u8, len: usize) callconv(.c) void {
    if (!spawned) return;
    pty.write(ptr[0..len]);
}

export fn anvil_frame(out: *inst.FrameData) callconv(.c) void {
    if (!ready) {
        out.count = 0;
        return;
    }
    var n = renderer.buildInstances(&term, instances[0..]);
    instances[n] = renderer.cursorInstance(&term);
    n += 1;
    out.* = .{
        .instances = &instances,
        .count = @intCast(n),
        .cell_w = renderer.cell_w,
        .cell_h = renderer.cell_h,
        .pad_x = renderer.pad_x,
        .pad_y = renderer.pad_y,
        .cell_uv = renderer.atlas.cellUV(),
    };
}
