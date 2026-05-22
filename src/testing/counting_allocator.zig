//! A std.mem.Allocator adapter that wraps a child allocator and either tallies
//! allocations (.count mode) or returns OOM after N allocations (.fail_after mode).
//!
//! Usage:
//!   var ca = CountingAllocator.init(child_allocator);
//!   var alloc = ca.allocator();
//!   ca.reset();
//!   // ca.alloc_count, ca.resize_count, ca.free_count

const std = @import("std");
const Alignment = std.mem.Alignment;

pub const Mode = union(enum) {
    /// Forward every call to the child and increment the counters.
    count,
    /// Forward the first N alloc calls; return error.OutOfMemory thereafter.
    fail_after: usize,
};

pub const CountingAllocator = struct {
    child: std.mem.Allocator,
    mode: Mode = .count,
    alloc_count: usize = 0,
    resize_count: usize = 0,
    free_count: usize = 0,

    pub fn init(child: std.mem.Allocator) CountingAllocator {
        return .{ .child = child };
    }

    pub fn allocator(self: *CountingAllocator) std.mem.Allocator {
        return .{ .ptr = self, .vtable = &vtable };
    }

    pub fn reset(self: *CountingAllocator) void {
        self.alloc_count = 0;
        self.resize_count = 0;
        self.free_count = 0;
    }

    fn alloc(ctx: *anyopaque, n: usize, alignment: Alignment, ra: usize) ?[*]u8 {
        const self: *CountingAllocator = @ptrCast(@alignCast(ctx));
        switch (self.mode) {
            .count => {
                self.alloc_count += 1;
                return self.child.rawAlloc(n, alignment, ra);
            },
            .fail_after => |limit| {
                if (self.alloc_count >= limit) return null;
                self.alloc_count += 1;
                return self.child.rawAlloc(n, alignment, ra);
            },
        }
    }

    fn resize(ctx: *anyopaque, buf: []u8, alignment: Alignment, new_len: usize, ra: usize) bool {
        const self: *CountingAllocator = @ptrCast(@alignCast(ctx));
        self.resize_count += 1;
        return self.child.rawResize(buf, alignment, new_len, ra);
    }

    fn remap(ctx: *anyopaque, buf: []u8, alignment: Alignment, new_len: usize, ra: usize) ?[*]u8 {
        const self: *CountingAllocator = @ptrCast(@alignCast(ctx));
        self.resize_count += 1;
        return self.child.rawRemap(buf, alignment, new_len, ra);
    }

    fn free(ctx: *anyopaque, buf: []u8, alignment: Alignment, ra: usize) void {
        const self: *CountingAllocator = @ptrCast(@alignCast(ctx));
        self.free_count += 1;
        self.child.rawFree(buf, alignment, ra);
    }

    const vtable: std.mem.Allocator.VTable = .{
        .alloc = alloc,
        .resize = resize,
        .remap = remap,
        .free = free,
    };
};

const testing = std.testing;

test "count mode tallies alloc/free" {
    var ca = CountingAllocator.init(testing.allocator);
    const alloc = ca.allocator();

    const buf = try alloc.alloc(u8, 64);
    try testing.expectEqual(@as(usize, 1), ca.alloc_count);
    try testing.expectEqual(@as(usize, 0), ca.free_count);

    alloc.free(buf);
    try testing.expectEqual(@as(usize, 1), ca.free_count);

    ca.reset();
    try testing.expectEqual(@as(usize, 0), ca.alloc_count);
    try testing.expectEqual(@as(usize, 0), ca.free_count);
}

test "fail_after returns OOM at N" {
    var ca = CountingAllocator.init(testing.allocator);
    ca.mode = .{ .fail_after = 2 };
    const alloc = ca.allocator();

    // First two succeed.
    const b1 = try alloc.alloc(u8, 8);
    const b2 = try alloc.alloc(u8, 8);
    try testing.expectEqual(@as(usize, 2), ca.alloc_count);

    // Third must fail.
    try testing.expectError(error.OutOfMemory, alloc.alloc(u8, 8));
    try testing.expectEqual(@as(usize, 2), ca.alloc_count); // did not increment

    alloc.free(b1);
    alloc.free(b2);
}
