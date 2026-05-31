const std = @import("std");

const c = @cImport({
    @cInclude("stdio.h");
    @cInclude("stdlib.h");
});

pub const size_cap: usize = 2 * 1024 * 1024; // 2 MiB
const binary_scan: usize = 1024;

pub const LoadResult = struct {
    bytes: []u8,
    truncated: bool,
    is_binary: bool,

    pub fn deinit(self: *LoadResult, alloc: std.mem.Allocator) void {
        alloc.free(self.bytes);
    }
};

pub fn load(alloc: std.mem.Allocator, path: []const u8) !LoadResult {
    var path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    if (path.len >= path_buf.len) return error.NameTooLong;
    @memcpy(path_buf[0..path.len], path);
    path_buf[path.len] = 0;

    const f = c.fopen(&path_buf, "rb") orelse return error.FileNotFound;
    defer _ = c.fclose(f);

    _ = c.fseek(f, 0, c.SEEK_END);
    const raw_sz = c.ftell(f);
    _ = c.fseek(f, 0, c.SEEK_SET);
    if (raw_sz < 0) return error.FileNotFound;

    const file_sz: usize = @intCast(raw_sz);
    const read_sz = @min(file_sz, size_cap);
    const truncated = file_sz > size_cap;

    const buf = try alloc.alloc(u8, read_sz);
    errdefer alloc.free(buf);

    const n = c.fread(buf.ptr, 1, read_sz, f);
    if (n != read_sz) {
        alloc.free(buf);
        return error.ReadError;
    }

    const scan_n = @min(n, binary_scan);
    const is_binary = std.mem.indexOfScalar(u8, buf[0..scan_n], 0) != null;

    return .{ .bytes = buf, .truncated = truncated, .is_binary = is_binary };
}

/// Write `bytes` to `path`, truncating any existing file.
pub fn save(path: []const u8, bytes: []const u8) !void {
    var path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    if (path.len >= path_buf.len) return error.NameTooLong;
    @memcpy(path_buf[0..path.len], path);
    path_buf[path.len] = 0;

    const f = c.fopen(&path_buf, "wb") orelse return error.OpenFailed;
    defer _ = c.fclose(f);

    if (bytes.len > 0) {
        const n = c.fwrite(bytes.ptr, 1, bytes.len, f);
        if (n != bytes.len) return error.WriteError;
    }
}

/// Split `bytes` into lines. Returns a slice of slices into `bytes` (no alloc).
/// Written into `out`; returns the count. Lines are separated by '\n'; the
/// terminator is not included. A trailing newline adds no empty line.
pub fn splitLines(bytes: []const u8, out: [][]const u8) usize {
    var n: usize = 0;
    var start: usize = 0;
    for (bytes, 0..) |b, i| {
        if (b == '\n') {
            if (n < out.len) {
                out[n] = bytes[start..i];
                n += 1;
            }
            start = i + 1;
        }
    }
    if (start < bytes.len and n < out.len) {
        out[n] = bytes[start..];
        n += 1;
    }
    return n;
}

test "binary detection: NUL byte flags binary" {
    const alloc = std.testing.allocator;
    const buf = try alloc.dupe(u8, &[_]u8{ 'h', 'e', 'l', 0, 'o' });
    const scan_n = @min(buf.len, binary_scan);
    const is_binary = std.mem.indexOfScalar(u8, buf[0..scan_n], 0) != null;
    alloc.free(buf);
    try std.testing.expect(is_binary);
}

test "binary detection: no NUL is not binary" {
    const alloc = std.testing.allocator;
    const buf = try alloc.dupe(u8, "hello world\n");
    const scan_n = @min(buf.len, binary_scan);
    const is_binary = std.mem.indexOfScalar(u8, buf[0..scan_n], 0) != null;
    alloc.free(buf);
    try std.testing.expect(!is_binary);
}

test "splitLines: basic line splitting" {
    const text = "line1\nline2\nline3";
    var lines: [16][]const u8 = undefined;
    const n = splitLines(text, &lines);
    try std.testing.expectEqual(@as(usize, 3), n);
    try std.testing.expectEqualStrings("line1", lines[0]);
    try std.testing.expectEqualStrings("line2", lines[1]);
    try std.testing.expectEqualStrings("line3", lines[2]);
}

test "splitLines: trailing newline does not add empty line" {
    const text = "a\nb\n";
    var lines: [16][]const u8 = undefined;
    const n = splitLines(text, &lines);
    try std.testing.expectEqual(@as(usize, 2), n);
    try std.testing.expectEqualStrings("a", lines[0]);
    try std.testing.expectEqualStrings("b", lines[1]);
}

test "splitLines: empty input" {
    var lines: [4][]const u8 = undefined;
    const n = splitLines("", &lines);
    try std.testing.expectEqual(@as(usize, 0), n);
}

test "load: file not found returns error" {
    const alloc = std.testing.allocator;
    const result = load(alloc, "/tmp/anvil_test_nonexistent_file_xyz.txt");
    try std.testing.expectError(error.FileNotFound, result);
}
