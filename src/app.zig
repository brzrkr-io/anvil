const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const Parser = @import("vt/parser.zig").Parser;
const Renderer = @import("render/renderer.zig").Renderer;
const inst = @import("render/instance.zig");

const shader_src = @embedFile("platform/shaders.metal");
const max_instances = 60000;
const font_pt: f32 = 13.0;

var term: Terminal = undefined;
var parser: Parser = .{};
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = 44 };
var instances: [max_instances]inst.CellInstance = undefined;
var ready = false;

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
    out.* = .{
        .first = a.first,
        .count = a.count,
        .cols = a.cols,
        .rows = a.rows(),
        .pt_size = font_pt,
    };
}

export fn anvil_set_metrics(cell_w: f32, cell_h: f32) callconv(.c) void {
    renderer.cell_w = cell_w;
    renderer.cell_h = cell_h;
}

export fn anvil_resize(px_w: f32, px_h: f32) callconv(.c) void {
    const g = renderer.gridSize(px_w, px_h);
    if (ready) {
        if (g.cols == term.grid.cols and g.rows == term.grid.rows) return;
        term.deinit();
    }
    term = Terminal.init(std.heap.page_allocator, g.rows, g.cols) catch return;
    ready = true;
    seedPattern();
}

export fn anvil_frame(out: *inst.FrameData) callconv(.c) void {
    if (!ready) {
        out.count = 0;
        return;
    }
    const n = renderer.buildInstances(&term, instances[0..]);
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

fn seedPattern() void {
    parser = .{};
    parser.feed(&term, "\x1b[1;32manvil\x1b[0m ready \x2d \x1b[31mM2.2\x1b[0m quads\r\n");
    parser.feed(&term, "\x1b[44m blue bg \x1b[0m \x1b[43;30m amber \x1b[0m\r\n");
}
