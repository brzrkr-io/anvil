//! File-tree model: a flattened, display-ordered list of visible filesystem
//! entries rooted at a directory. Expand/collapse dirs in-place. No allocator;
//! uses a fixed-size array bounded by `max_entries`.
//!
//! Directory reads use std.c POSIX calls (opendir/readdir/closedir) to avoid
//! the Zig 0.16 high-level fs API that requires an std.Io context.

const std = @import("std");

/// Maximum number of visible entries. A giant tree stops adding past this cap.
pub const max_entries: usize = 2000;

/// Maximum byte length of a single entry name (excluding null terminator).
pub const max_name: usize = 255;

pub const Entry = struct {
    name: [max_name + 1]u8 = undefined,
    name_len: usize = 0,
    /// Absolute path of this entry on disk.
    path: [std.fs.max_path_bytes]u8 = undefined,
    path_len: usize = 0,
    depth: u16 = 0,
    is_dir: bool = false,
    expanded: bool = false,

    pub fn nameSlice(self: *const Entry) []const u8 {
        return self.name[0..self.name_len];
    }

    pub fn pathSlice(self: *const Entry) []const u8 {
        return self.path[0..self.path_len];
    }
};

pub const FileTree = struct {
    entries: [max_entries]Entry = undefined,
    count: usize = 0,
    /// Index of the currently selected/open entry, or null if none.
    selected_idx: ?usize = null,

    /// Root the tree at `root_path` and load its immediate children.
    /// The root entry itself is not shown; children are at depth 0.
    pub fn setRoot(self: *FileTree, root_path: []const u8) void {
        self.count = 0;
        self.selected_idx = null;
        loadChildren(self, root_path, 0);
    }

    /// Toggle expand/collapse of the entry at visible index `idx`.
    /// Expanding reads that directory's children and splices them in at depth+1.
    /// Collapsing removes all descendants (entries with depth > entry.depth
    /// that follow consecutively).
    pub fn toggle(self: *FileTree, idx: usize) void {
        if (idx >= self.count) return;
        const e = &self.entries[idx];
        if (!e.is_dir) return;

        if (e.expanded) {
            // Collapse: remove all immediately following entries whose depth
            // is greater than this entry's depth.
            e.expanded = false;
            const base_depth = e.depth;
            var end = idx + 1;
            while (end < self.count and self.entries[end].depth > base_depth) {
                end += 1;
            }
            // Shift entries after `end` to fill the gap.
            const remove_count = end - (idx + 1);
            if (remove_count > 0) {
                var i = idx + 1;
                while (i + remove_count < self.count) : (i += 1) {
                    self.entries[i] = self.entries[i + remove_count];
                }
                self.count -= remove_count;
            }
        } else {
            // Expand: load children into a temporary scratch area, then splice.
            e.expanded = true;
            const path = e.pathSlice();
            const insert_at = idx + 1;
            const child_depth: u16 = e.depth + 1;

            // Count existing entries after insert_at (they need to shift right).
            const tail_count = self.count - insert_at;

            // Temporary storage for new children (on the stack).
            var scratch: [max_entries]Entry = undefined;
            var scratch_count: usize = 0;
            collectChildren(path, child_depth, &scratch, &scratch_count);

            if (scratch_count == 0) return;

            // How many new entries can we actually fit?
            const available = if (self.count + scratch_count <= max_entries)
                scratch_count
            else
                max_entries - self.count;
            if (available == 0) return;

            // Shift tail right to make room.
            if (tail_count > 0) {
                var i = self.count + available - 1;
                const stop = insert_at + available;
                while (i >= stop) : (i -= 1) {
                    self.entries[i] = self.entries[i - available];
                    if (i == 0) break;
                }
            }

            // Copy the children in.
            for (0..available) |j| {
                self.entries[insert_at + j] = scratch[j];
            }
            self.count += available;
        }
    }
};

// --- internal helpers -------------------------------------------------------

/// Temporary sort buffer: holds one directory's children before insertion.
const SortEntry = struct {
    name: [max_name + 1]u8,
    name_len: usize,
    path: [std.fs.max_path_bytes]u8,
    path_len: usize,
    is_dir: bool,
};

/// Read the immediate children of `dir_path` and append them into `entries`
/// starting at `*count`. Dirs sort before files; each group alphabetically.
fn loadChildren(tree: *FileTree, dir_path: []const u8, depth: u16) void {
    var scratch: [max_entries]SortEntry = undefined;
    var scratch_count: usize = 0;
    collectChildrenInto(dir_path, &scratch, &scratch_count);
    for (0..scratch_count) |i| {
        if (tree.count >= max_entries) break;
        const s = &scratch[i];
        var e: Entry = .{
            .name_len = s.name_len,
            .path_len = s.path_len,
            .depth = depth,
            .is_dir = s.is_dir,
            .expanded = false,
        };
        @memcpy(e.name[0..s.name_len], s.name[0..s.name_len]);
        @memcpy(e.path[0..s.path_len], s.path[0..s.path_len]);
        tree.entries[tree.count] = e;
        tree.count += 1;
    }
}

/// Read the immediate children of `dir_path` into `out[0..*count]`.
fn collectChildren(
    dir_path: []const u8,
    depth: u16,
    out: *[max_entries]Entry,
    count: *usize,
) void {
    var scratch: [max_entries]SortEntry = undefined;
    var scratch_count: usize = 0;
    collectChildrenInto(dir_path, &scratch, &scratch_count);
    for (0..scratch_count) |i| {
        if (count.* >= max_entries) break;
        const s = &scratch[i];
        var e: Entry = .{
            .name_len = s.name_len,
            .path_len = s.path_len,
            .depth = depth,
            .is_dir = s.is_dir,
            .expanded = false,
        };
        @memcpy(e.name[0..s.name_len], s.name[0..s.name_len]);
        @memcpy(e.path[0..s.path_len], s.path[0..s.path_len]);
        out[count.*] = e;
        count.* += 1;
    }
}

/// Read `dir_path` with opendir/readdir and fill `scratch[0..*count]`,
/// sorted dirs-first then files, each group alphabetically.
fn collectChildrenInto(
    dir_path: []const u8,
    scratch: *[max_entries]SortEntry,
    count: *usize,
) void {
    count.* = 0;
    if (dir_path.len == 0) return;

    // Null-terminate the path.
    var path_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    if (dir_path.len >= path_z_buf.len) return;
    @memcpy(path_z_buf[0..dir_path.len], dir_path);
    path_z_buf[dir_path.len] = 0;
    const path_z: [*:0]const u8 = path_z_buf[0..dir_path.len :0];

    const dir = std.c.opendir(path_z) orelse return;
    defer _ = std.c.closedir(dir);

    // Separate dirs and files, then merge (dirs first).
    var dirs: [max_entries]SortEntry = undefined;
    var dir_count: usize = 0;
    var files: [max_entries]SortEntry = undefined;
    var file_count: usize = 0;

    while (std.c.readdir(dir)) |de| {
        // On macOS, dirent has `.name` (not `d_name`) and `.namlen`.
        const raw_name = de.name[0..de.namlen];
        // Skip . and ..
        if (std.mem.eql(u8, raw_name, ".") or std.mem.eql(u8, raw_name, "..")) continue;
        const name_len = @min(raw_name.len, max_name);
        if (name_len == 0) continue;

        // Build absolute path.
        var child_path_buf: [std.fs.max_path_bytes]u8 = undefined;
        const child_path = std.fmt.bufPrint(&child_path_buf, "{s}/{s}", .{ dir_path, raw_name[0..name_len] }) catch continue;
        if (child_path.len > std.fs.max_path_bytes) continue;

        const is_dir = de.type == std.c.DT.DIR;

        var se: SortEntry = .{
            .name = undefined,
            .name_len = name_len,
            .path = undefined,
            .path_len = child_path.len,
            .is_dir = is_dir,
        };
        @memcpy(se.name[0..name_len], raw_name[0..name_len]);
        @memcpy(se.path[0..child_path.len], child_path);

        if (is_dir) {
            if (dir_count < max_entries) {
                dirs[dir_count] = se;
                dir_count += 1;
            }
        } else {
            if (file_count < max_entries) {
                files[file_count] = se;
                file_count += 1;
            }
        }
    }

    // Sort each group alphabetically (insertion sort — small N, simple).
    sortSortEntries(dirs[0..dir_count]);
    sortSortEntries(files[0..file_count]);

    // Merge dirs then files into scratch.
    for (0..dir_count) |i| {
        if (count.* >= max_entries) break;
        scratch[count.*] = dirs[i];
        count.* += 1;
    }
    for (0..file_count) |i| {
        if (count.* >= max_entries) break;
        scratch[count.*] = files[i];
        count.* += 1;
    }
}

/// Insertion sort on a slice of SortEntry by name (case-sensitive, lexicographic).
fn sortSortEntries(entries: []SortEntry) void {
    var i: usize = 1;
    while (i < entries.len) : (i += 1) {
        const key = entries[i];
        var j = i;
        while (j > 0) {
            const a = entries[j - 1].name[0..entries[j - 1].name_len];
            const b = key.name[0..key.name_len];
            if (std.mem.order(u8, a, b) != .gt) break;
            entries[j] = entries[j - 1];
            j -= 1;
        }
        entries[j] = key;
    }
}

// --- Tests ------------------------------------------------------------------

const testing = std.testing;

test "FileTree setRoot on a temp directory" {
    // Create a temp dir with known contents.
    var tmp_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp = std.fmt.bufPrint(&tmp_buf, "/tmp/filetree_test_{d}", .{std.c.getpid()}) catch return;
    const tmp_z: [*:0]const u8 = blk: {
        var zb: [std.fs.max_path_bytes + 1]u8 = undefined;
        @memcpy(zb[0..tmp.len], tmp);
        zb[tmp.len] = 0;
        break :blk zb[0..tmp.len :0];
    };
    _ = std.c.mkdir(tmp_z, 0o755);
    defer _ = std.c.rmdir(tmp_z);

    // Create a subdir and a file.
    var sub_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const sub = std.fmt.bufPrint(sub_buf[0..std.fs.max_path_bytes], "{s}/subdir", .{tmp}) catch return;
    sub_buf[sub.len] = 0;
    const sub_z: [*:0]const u8 = sub_buf[0..sub.len :0];
    _ = std.c.mkdir(sub_z, 0o755);
    defer _ = std.c.rmdir(sub_z);

    var file_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const file_path = std.fmt.bufPrint(file_buf[0..std.fs.max_path_bytes], "{s}/hello.txt", .{tmp}) catch return;
    file_buf[file_path.len] = 0;
    const file_z: [*:0]const u8 = file_buf[0..file_path.len :0];
    const fd = std.c.open(file_z, .{ .CREAT = true, .ACCMODE = .WRONLY }, @as(c_uint, 0o644));
    if (fd >= 0) _ = std.c.close(fd);
    defer _ = std.c.unlink(file_z);

    var tree: FileTree = .{};
    tree.setRoot(tmp);

    // Should have 2 entries: subdir (dir) first, hello.txt (file) second.
    try testing.expectEqual(@as(usize, 2), tree.count);
    try testing.expectEqualStrings("subdir", tree.entries[0].nameSlice());
    try testing.expect(tree.entries[0].is_dir);
    try testing.expectEqualStrings("hello.txt", tree.entries[1].nameSlice());
    try testing.expect(!tree.entries[1].is_dir);
}

test "FileTree toggle expand then collapse" {
    var tmp_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp = std.fmt.bufPrint(&tmp_buf, "/tmp/filetree_toggle_{d}", .{std.c.getpid()}) catch return;
    var tmp_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    @memcpy(tmp_z_buf[0..tmp.len], tmp);
    tmp_z_buf[tmp.len] = 0;
    const tmp_z: [*:0]const u8 = tmp_z_buf[0..tmp.len :0];
    _ = std.c.mkdir(tmp_z, 0o755);
    defer _ = std.c.rmdir(tmp_z);

    // subdir/child.txt
    var sub_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const sub = std.fmt.bufPrint(sub_buf[0..std.fs.max_path_bytes], "{s}/subdir", .{tmp}) catch return;
    sub_buf[sub.len] = 0;
    const sub_z: [*:0]const u8 = sub_buf[0..sub.len :0];
    _ = std.c.mkdir(sub_z, 0o755);
    defer _ = std.c.rmdir(sub_z);

    var child_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const child_path = std.fmt.bufPrint(child_buf[0..std.fs.max_path_bytes], "{s}/subdir/child.txt", .{tmp}) catch return;
    child_buf[child_path.len] = 0;
    const child_z: [*:0]const u8 = child_buf[0..child_path.len :0];
    const fd = std.c.open(child_z, .{ .CREAT = true, .ACCMODE = .WRONLY }, @as(c_uint, 0o644));
    if (fd >= 0) _ = std.c.close(fd);
    defer _ = std.c.unlink(child_z);

    var tree: FileTree = .{};
    tree.setRoot(tmp);
    try testing.expectEqual(@as(usize, 1), tree.count); // just subdir

    // Expand subdir.
    tree.toggle(0);
    try testing.expectEqual(@as(usize, 2), tree.count); // subdir + child.txt
    try testing.expect(tree.entries[0].expanded);
    try testing.expectEqualStrings("child.txt", tree.entries[1].nameSlice());
    try testing.expectEqual(@as(u16, 1), tree.entries[1].depth);

    // Collapse subdir.
    tree.toggle(0);
    try testing.expectEqual(@as(usize, 1), tree.count);
    try testing.expect(!tree.entries[0].expanded);
}

test "FileTree dirs sort before files" {
    var tmp_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp = std.fmt.bufPrint(&tmp_buf, "/tmp/filetree_sort_{d}", .{std.c.getpid()}) catch return;
    var tmp_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    @memcpy(tmp_z_buf[0..tmp.len], tmp);
    tmp_z_buf[tmp.len] = 0;
    const tmp_z: [*:0]const u8 = tmp_z_buf[0..tmp.len :0];
    _ = std.c.mkdir(tmp_z, 0o755);
    defer _ = std.c.rmdir(tmp_z);

    // Create: z_dir (dir), a_file.txt (file), a_dir (dir), z_file.txt (file).
    const names = [_][]const u8{ "z_dir", "a_dir" };
    const file_names = [_][]const u8{ "z_file.txt", "a_file.txt" };

    for (names) |n| {
        var b: [std.fs.max_path_bytes + 1]u8 = undefined;
        const p = std.fmt.bufPrint(b[0..std.fs.max_path_bytes], "{s}/{s}", .{ tmp, n }) catch continue;
        b[p.len] = 0;
        _ = std.c.mkdir(b[0..p.len :0], 0o755);
    }
    for (file_names) |n| {
        var b: [std.fs.max_path_bytes + 1]u8 = undefined;
        const p = std.fmt.bufPrint(b[0..std.fs.max_path_bytes], "{s}/{s}", .{ tmp, n }) catch continue;
        b[p.len] = 0;
        const fd = std.c.open(b[0..p.len :0], .{ .CREAT = true, .ACCMODE = .WRONLY }, @as(c_uint, 0o644));
        if (fd >= 0) _ = std.c.close(fd);
    }
    defer {
        for (names) |n| {
            var b: [std.fs.max_path_bytes + 1]u8 = undefined;
            const p = std.fmt.bufPrint(b[0..std.fs.max_path_bytes], "{s}/{s}", .{ tmp, n }) catch continue;
            b[p.len] = 0;
            _ = std.c.rmdir(b[0..p.len :0]);
        }
        for (file_names) |n| {
            var b: [std.fs.max_path_bytes + 1]u8 = undefined;
            const p = std.fmt.bufPrint(b[0..std.fs.max_path_bytes], "{s}/{s}", .{ tmp, n }) catch continue;
            b[p.len] = 0;
            _ = std.c.unlink(b[0..p.len :0]);
        }
        _ = std.c.rmdir(tmp_z);
    }

    var tree: FileTree = .{};
    tree.setRoot(tmp);
    try testing.expectEqual(@as(usize, 4), tree.count);
    // First two must be directories.
    try testing.expect(tree.entries[0].is_dir);
    try testing.expect(tree.entries[1].is_dir);
    // Dirs must be alphabetically sorted.
    const order = std.mem.order(u8, tree.entries[0].nameSlice(), tree.entries[1].nameSlice());
    try testing.expect(order == .lt or order == .eq);
    // Last two must be files.
    try testing.expect(!tree.entries[2].is_dir);
    try testing.expect(!tree.entries[3].is_dir);
}

test "FileTree max_entries cap is enforced" {
    // Use /usr/lib if available; otherwise /usr/bin. Either has >> 2000 entries.
    // We just check the count never exceeds max_entries.
    var tree: FileTree = .{};
    // Root at a known large directory; if unavailable the count stays 0.
    tree.setRoot("/usr/lib");
    try testing.expect(tree.count <= max_entries);
}

test "toggle on a non-dir is a no-op" {
    var tmp_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp = std.fmt.bufPrint(&tmp_buf, "/tmp/filetree_noop_{d}", .{std.c.getpid()}) catch return;
    var tmp_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    @memcpy(tmp_z_buf[0..tmp.len], tmp);
    tmp_z_buf[tmp.len] = 0;
    const tmp_z: [*:0]const u8 = tmp_z_buf[0..tmp.len :0];
    _ = std.c.mkdir(tmp_z, 0o755);
    defer _ = std.c.rmdir(tmp_z);

    var file_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const fp = std.fmt.bufPrint(file_buf[0..std.fs.max_path_bytes], "{s}/f.txt", .{tmp}) catch return;
    file_buf[fp.len] = 0;
    const fd = std.c.open(file_buf[0..fp.len :0], .{ .CREAT = true, .ACCMODE = .WRONLY }, @as(c_uint, 0o644));
    if (fd >= 0) _ = std.c.close(fd);
    defer _ = std.c.unlink(file_buf[0..fp.len :0]);

    var tree: FileTree = .{};
    tree.setRoot(tmp);
    try testing.expectEqual(@as(usize, 1), tree.count);
    tree.toggle(0); // file, not dir — no-op
    try testing.expectEqual(@as(usize, 1), tree.count);
}
