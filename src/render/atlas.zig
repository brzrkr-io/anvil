const std = @import("std");

/// Glyph atlas layout. The shim rasterizes glyphs `first..first+count` into a
/// `cols`-wide grid texture; this maps a codepoint to its normalized UV origin.
pub const Atlas = struct {
    first: u21 = 32,
    count: u16 = 95,
    cols: u16 = 16,

    pub fn rows(self: Atlas) u16 {
        return (self.count + self.cols - 1) / self.cols;
    }

    pub fn cellUV(self: Atlas) [2]f32 {
        return .{
            1.0 / @as(f32, @floatFromInt(self.cols)),
            1.0 / @as(f32, @floatFromInt(self.rows())),
        };
    }

    /// UV of the top-left corner of `cp`'s atlas cell. Out-of-range codepoints
    /// map to the first slot (the space glyph), rendering blank.
    pub fn uvOrigin(self: Atlas, cp: u21) [2]f32 {
        const idx: u16 = if (cp >= self.first and cp < self.first + self.count)
            @intCast(cp - self.first)
        else
            0;
        const col = idx % self.cols;
        const row = idx / self.cols;
        return .{
            @as(f32, @floatFromInt(col)) / @as(f32, @floatFromInt(self.cols)),
            @as(f32, @floatFromInt(row)) / @as(f32, @floatFromInt(self.rows())),
        };
    }
};

test "rows is ceil(count/cols)" {
    try std.testing.expectEqual(@as(u16, 6), (Atlas{ .count = 95, .cols = 16 }).rows());
    try std.testing.expectEqual(@as(u16, 1), (Atlas{ .count = 16, .cols = 16 }).rows());
    try std.testing.expectEqual(@as(u16, 2), (Atlas{ .count = 17, .cols = 16 }).rows());
}

test "uvOrigin maps codepoint to grid cell" {
    const a = Atlas{ .first = 32, .count = 95, .cols = 16 };
    // 'A' (65) -> idx 33 -> col 1, row 2
    const uv = a.uvOrigin('A');
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 16.0), uv[0], 0.0001);
    try std.testing.expectApproxEqAbs(@as(f32, 2.0 / 6.0), uv[1], 0.0001);
}

test "uvOrigin first slot for space and out-of-range" {
    const a = Atlas{};
    try std.testing.expectEqual([2]f32{ 0, 0 }, a.uvOrigin(' '));
    try std.testing.expectEqual([2]f32{ 0, 0 }, a.uvOrigin(0x1F600)); // emoji, unsupported
    try std.testing.expectEqual([2]f32{ 0, 0 }, a.uvOrigin(0)); // control
}

test "cellUV is one grid cell" {
    const a = Atlas{ .count = 95, .cols = 16 };
    const uv = a.cellUV();
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 16.0), uv[0], 0.0001);
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 6.0), uv[1], 0.0001);
}
