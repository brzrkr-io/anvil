//! The atomic unit of the terminal grid: one character cell, plus the color
//! and attribute types it carries. Pure data — no behavior, no allocation.

const std = @import("std");

/// A foreground or background color. `default` defers to the renderer's
/// theme; `palette` indexes the 256-color table; `rgb` is 24-bit truecolor.
pub const Color = union(enum) {
    default,
    palette: u8,
    rgb: [3]u8,
};

/// The eight SGR rendition flags. Packed so a `Cell` stays small.
pub const Attrs = packed struct {
    bold: bool = false,
    dim: bool = false,
    italic: bool = false,
    underline: bool = false,
    blink: bool = false,
    inverse: bool = false,
    invisible: bool = false,
    strikethrough: bool = false,
};

/// One grid cell: a Unicode scalar plus its rendition. A freshly-default
/// `Cell` is a blank space with the theme colors and no attributes.
pub const Cell = struct {
    cp: u21 = ' ',
    fg: Color = .default,
    bg: Color = .default,
    attrs: Attrs = .{},

    /// True when this cell is visually identical to a default blank — used by
    /// scrollback row trimming to drop trailing empty cells.
    pub fn isBlank(self: Cell) bool {
        return std.meta.eql(self, Cell{});
    }
};

test "default cell is a blank space with theme colors" {
    const cell = Cell{};
    try std.testing.expectEqual(@as(u21, ' '), cell.cp);
    try std.testing.expectEqual(Color.default, cell.fg);
    try std.testing.expectEqual(Color.default, cell.bg);
    try std.testing.expect(cell.isBlank());
}

test "isBlank distinguishes content and rendition" {
    var cell = Cell{};
    cell.cp = 'x';
    try std.testing.expect(!cell.isBlank());

    var colored = Cell{};
    colored.bg = .{ .palette = 4 };
    try std.testing.expect(!colored.isBlank());

    var attributed = Cell{};
    attributed.attrs.bold = true;
    try std.testing.expect(!attributed.isBlank());
}

test "Color variants compare by value" {
    try std.testing.expect(std.meta.eql(Color{ .palette = 7 }, Color{ .palette = 7 }));
    try std.testing.expect(!std.meta.eql(Color{ .palette = 7 }, Color{ .palette = 8 }));
    try std.testing.expect(std.meta.eql(Color{ .rgb = .{ 1, 2, 3 } }, Color{ .rgb = .{ 1, 2, 3 } }));
    try std.testing.expect(!std.meta.eql(Color.default, Color{ .rgb = .{ 0, 0, 0 } }));
}

test "Attrs defaults are all false" {
    const attrs = Attrs{};
    inline for (std.meta.fields(Attrs)) |field| {
        try std.testing.expect(!@field(attrs, field.name));
    }
}
