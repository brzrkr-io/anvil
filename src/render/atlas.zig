const std = @import("std");

/// Cache geometry. `capacity` must be a power of two (cheap hash masking) and
/// equal `cols * rows_n`. The shim allocates a `cols`x`rows_n` grid texture.
pub const cols: u16 = 32;
pub const rows_n: u16 = 32;
pub const capacity: u16 = cols * rows_n; // 1024 slots

/// Key bit that tags a slot as belonging to the IBM Plex Sans face rather than
/// the primary mono font. The shim strips this bit and rasterizes the low 21
/// bits with the Sans CTFont. Keeps Sans glyphs from colliding with the mono
/// glyph of the same codepoint in the shared atlas grid.
pub const sans_tag: u32 = 0x0020_0000; // bit 21 (above the 0x10FFFF cp range)

/// Key bit that tags a slot as a small chrome glyph rasterized at the reduced
/// chrome scale (mono face, top-left aligned in the cell). Keeps the small
/// chrome icon/dot from colliding with the full-size terminal glyph of the same
/// codepoint. Stripped by the shim before rasterizing.
pub const chrome_tag: u32 = 0x0040_0000; // bit 22

/// One glyph the shim must rasterize this frame: `cp` into grid `slot`.
/// `wide` = 1 means a double-width glyph spanning `slot` and `slot`+1.
/// Layout matches the C `PendingGlyph` read in shim.m (three u32s).
pub const PendingGlyph = extern struct { cp: u32, slot: u32, wide: u32 = 0 };

/// Dynamic glyph atlas. Maps each codepoint to a slot in the grid texture,
/// rasterizing on first use (lazy). Slot 0 is the blank cell, shared by space
/// and control codepoints. When the cache fills, it is flushed at the next
/// frame boundary (see resetPending) so the live working set re-rasterizes
/// instead of new codepoints degrading to blank forever.
///
/// Open-addressing hash, no allocator: the table lives inline so the atlas is a
/// plain value with comptime-zeroed buckets. `keys[i] == 0` marks an empty
/// bucket; real keys are always > ' ' since blanks short-circuit to slot 0.
pub const Atlas = struct {
    keys: [capacity]u32 = [_]u32{0} ** capacity,
    vals: [capacity]u16 = undefined,
    next: u16 = 1, // slot 0 reserved for blank
    pending: [capacity]PendingGlyph = undefined,
    pending_n: u16 = 0,

    pub fn rows(_: *const Atlas) u16 {
        return rows_n;
    }

    /// Size of one glyph cell in normalized UV.
    pub fn cellUV(_: *const Atlas) [2]f32 {
        return .{
            1.0 / @as(f32, @floatFromInt(cols)),
            1.0 / @as(f32, @floatFromInt(rows_n)),
        };
    }

    /// Top-left UV of `slot`'s cell in the grid texture.
    pub fn slotUV(slot: u16) [2]f32 {
        return .{
            @as(f32, @floatFromInt(slot % cols)) / @as(f32, @floatFromInt(cols)),
            @as(f32, @floatFromInt(slot / cols)) / @as(f32, @floatFromInt(rows_n)),
        };
    }

    /// Slot for `cp`, assigning one and queueing a raster on first use. Space
    /// and control codepoints share the blank slot 0. Returns 0 when the cache
    /// is full.
    pub fn slotFor(self: *Atlas, cp: u21) u16 {
        if (cp <= ' ') return 0;
        return self.slotForKey(@as(u32, cp));
    }

    /// Slot for an arbitrary atlas key, assigning + queueing a raster on first
    /// use. The key is the bare codepoint for the mono face, or `sans_tag | cp`
    /// for the Plex Sans face. No blank short-circuit: callers handle spaces.
    pub fn slotForKey(self: *Atlas, key: u32) u16 {
        const mask: u32 = capacity - 1;
        var i: u32 = (key *% 2654435761) & mask;
        while (self.keys[i] != 0) {
            if (self.keys[i] == key) return self.vals[i];
            i = (i + 1) & mask;
        }
        if (self.next >= capacity) return 0; // full → blank
        const slot = self.next;
        self.next += 1;
        self.keys[i] = key;
        self.vals[i] = slot;
        self.pending[self.pending_n] = .{ .cp = key, .slot = slot, .wide = 0 };
        self.pending_n += 1;
        return slot;
    }

    /// Left slot for a double-width `cp`, reserving two horizontally adjacent
    /// slots in the same texture row and queueing a wide raster. The right half
    /// lives at the returned slot + 1. Returns 0 (blank) when the cache is full.
    pub fn wideSlot(self: *Atlas, cp: u21) u16 {
        if (cp <= ' ') return 0;
        const mask: u32 = capacity - 1;
        var i: u32 = (@as(u32, cp) *% 2654435761) & mask;
        while (self.keys[i] != 0) {
            if (self.keys[i] == cp) return self.vals[i];
            i = (i + 1) & mask;
        }
        // Keep both halves in one row: skip a trailing last-column slot.
        if (self.next % cols == cols - 1) self.next += 1;
        if (self.next + 1 >= capacity) return 0; // no room for the pair
        const left = self.next;
        self.next += 2;
        self.keys[i] = cp;
        self.vals[i] = left;
        self.pending[self.pending_n] = .{ .cp = cp, .slot = left, .wide = 1 };
        self.pending_n += 1;
        return left;
    }

    /// UV of the top-left of `cp`'s cell, assigning + queueing on first use.
    pub fn uvOrigin(self: *Atlas, cp: u21) [2]f32 {
        return slotUV(self.slotFor(cp));
    }

    /// Drop the pending list. Called once at the start of each frame, before
    /// any glyph lookups, so `pending` holds only this frame's new codepoints.
    /// If the cache is (near) full, flush it here at the frame boundary: this
    /// frame's lookups then re-assign and re-raster the live working set, so a
    /// churning glyph set never gets permanently stuck on the blank slot.
    pub fn resetPending(self: *Atlas) void {
        self.pending_n = 0;
        if (self.next >= capacity - 1) {
            self.keys = [_]u32{0} ** capacity;
            self.next = 1;
        }
    }
};

test "blank and controls map to slot 0 without queueing" {
    var a = Atlas{};
    try std.testing.expectEqual(@as(u16, 0), a.slotFor(' '));
    try std.testing.expectEqual(@as(u16, 0), a.slotFor('\n'));
    try std.testing.expectEqual(@as(u16, 0), a.slotFor(0));
    try std.testing.expectEqual(@as(u16, 0), a.pending_n);
}

test "first printable gets slot 1 and queues a raster" {
    var a = Atlas{};
    try std.testing.expectEqual(@as(u16, 1), a.slotFor('A'));
    try std.testing.expectEqual(@as(u16, 1), a.pending_n);
    try std.testing.expectEqual(@as(u32, 'A'), a.pending[0].cp);
    try std.testing.expectEqual(@as(u32, 1), a.pending[0].slot);
}

test "repeat lookup is cached, distinct codepoints get distinct slots" {
    var a = Atlas{};
    const s1 = a.slotFor('A');
    const s2 = a.slotFor('A');
    const s3 = a.slotFor('B');
    try std.testing.expectEqual(s1, s2);
    try std.testing.expect(s1 != s3);
    try std.testing.expectEqual(@as(u16, 2), a.pending_n); // A, B — A not requeued
}

test "non-ASCII codepoints are cached like any other" {
    var a = Atlas{};
    const dot = a.slotFor(0x25CF); // ●
    const check = a.slotFor(0x2713); // ✓
    try std.testing.expect(dot != 0);
    try std.testing.expect(check != 0);
    try std.testing.expect(dot != check);
}

test "sans-tagged key gets a distinct slot from the mono codepoint" {
    var a = Atlas{};
    const mono_s = a.slotFor('S');
    const sans_s = a.slotForKey(sans_tag | 'S');
    try std.testing.expect(mono_s != 0);
    try std.testing.expect(sans_s != 0);
    try std.testing.expect(mono_s != sans_s); // same glyph, different face
    try std.testing.expectEqual(sans_s, a.slotForKey(sans_tag | 'S')); // cached
    try std.testing.expectEqual(sans_tag | @as(u32, 'S'), a.pending[1].cp);
}

test "resetPending clears the queue but keeps the cache" {
    var a = Atlas{};
    const s = a.slotFor('A');
    a.resetPending();
    try std.testing.expectEqual(@as(u16, 0), a.pending_n);
    try std.testing.expectEqual(s, a.slotFor('A')); // still cached
    try std.testing.expectEqual(@as(u16, 0), a.pending_n); // no requeue
}

test "cache full falls back to blank, then flushes at frame boundary" {
    var a = Atlas{};
    // Fill every slot past 0 with distinct codepoints.
    var cp: u21 = '!';
    while (a.next < capacity) : (cp += 1) _ = a.slotFor(cp);
    try std.testing.expectEqual(capacity, a.next);
    try std.testing.expectEqual(@as(u16, 0), a.slotFor(0x10FFFF)); // overflow → blank
    a.resetPending(); // frame boundary: cache was full, so it flushes
    try std.testing.expectEqual(@as(u16, 1), a.next);
    try std.testing.expectEqual(@as(u16, 1), a.slotFor(0x10FFFF)); // now allocatable
}

test "wideSlot reserves two adjacent slots in one row, cached, wide-flagged" {
    var a = Atlas{};
    const left = a.wideSlot(0x4E00); // CJK 一
    try std.testing.expect(left != 0);
    try std.testing.expectEqual(left % cols, (left + 1) % cols - 1); // same row, adjacent
    try std.testing.expectEqual(left, a.wideSlot(0x4E00)); // cached
    try std.testing.expectEqual(@as(u16, 1), a.pending_n); // queued once
    try std.testing.expectEqual(@as(u32, 1), a.pending[0].wide);
    try std.testing.expectEqual(left, @as(u16, @intCast(a.pending[0].slot)));
}

test "cellUV is one grid cell" {
    const a = Atlas{};
    const uv = a.cellUV();
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 32.0), uv[0], 0.0001);
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 32.0), uv[1], 0.0001);
}

test "uvOrigin places slot in the grid" {
    var a = Atlas{};
    _ = a.slotFor('A'); // slot 1 -> col 1, row 0
    const uv = a.uvOrigin('A');
    try std.testing.expectApproxEqAbs(@as(f32, 1.0 / 32.0), uv[0], 0.0001);
    try std.testing.expectApproxEqAbs(@as(f32, 0.0), uv[1], 0.0001);
}
