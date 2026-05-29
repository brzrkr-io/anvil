const std = @import("std");
const pane = @import("workspace/pane_tree.zig");
const SessionManager = @import("session_manager.zig").SessionManager;

const c = @cImport({
    @cInclude("stdio.h");
    @cInclude("stdlib.h");
    @cInclude("sys/stat.h");
});

// --- Serializable layout model ---

pub const NodeJson = union(enum) {
    leaf: LeafJson,
    split: SplitJson,
};

pub const LeafJson = struct {
    cwd: []const u8,
};

pub const SplitJson = struct {
    axis: u8,
    ratio: f32,
    a: *NodeJson,
    b: *NodeJson,
};

pub const TabJson = struct {
    root: NodeJson,
};

pub const StateJson = struct {
    version: u32,
    active_tab: usize,
    tabs: []const TabJson,
};

// --- Capture the live state from the session manager ---

pub fn capture(alloc: std.mem.Allocator, mgr: *const SessionManager) !StateJson {
    var tabs: std.ArrayListUnmanaged(TabJson) = .empty;
    for (mgr.tabs.items) |*tree| {
        const exported = try tree.exportRoot(alloc);
        defer pane.PaneTree.freeExport(alloc, exported);
        const node = try convertNode(alloc, exported, mgr);
        try tabs.append(alloc, TabJson{ .root = node });
    }
    return .{
        .version = 1,
        .active_tab = mgr.active_tab,
        .tabs = try tabs.toOwnedSlice(alloc),
    };
}

fn convertNode(alloc: std.mem.Allocator, node: pane.NodeExport, mgr: *const SessionManager) !NodeJson {
    switch (node) {
        .leaf => |id| {
            const cwd_str = if (mgr.byIdConst(id)) |s| s.term.cwd() else "";
            return .{ .leaf = .{ .cwd = try alloc.dupe(u8, cwd_str) } };
        },
        .split => |sp| {
            const a_ptr = try alloc.create(NodeJson);
            a_ptr.* = try convertNode(alloc, sp.a.*, mgr);
            const b_ptr = try alloc.create(NodeJson);
            b_ptr.* = try convertNode(alloc, sp.b.*, mgr);
            return .{ .split = .{
                .axis = @intFromEnum(sp.axis),
                .ratio = sp.ratio,
                .a = a_ptr,
                .b = b_ptr,
            } };
        },
    }
}

// --- Write JSON ---

pub fn writeJson(alloc: std.mem.Allocator, state: StateJson) ![]u8 {
    var buf: std.ArrayListUnmanaged(u8) = .empty;
    var tmp: [64]u8 = undefined;
    const hdr = try std.fmt.bufPrint(&tmp, "{{\"version\":{d},\"active_tab\":{d},\"tabs\":[", .{ state.version, state.active_tab });
    try buf.appendSlice(alloc, hdr);
    for (state.tabs, 0..) |tab, i| {
        if (i > 0) try buf.append(alloc, ',');
        try buf.appendSlice(alloc, "{\"root\":");
        try writeNodeJson(alloc, &buf, tab.root);
        try buf.append(alloc, '}');
    }
    try buf.appendSlice(alloc, "]}");
    return buf.toOwnedSlice(alloc);
}

fn writeNodeJson(alloc: std.mem.Allocator, buf: *std.ArrayListUnmanaged(u8), node: NodeJson) !void {
    var tmp: [64]u8 = undefined;
    switch (node) {
        .leaf => |lf| {
            try buf.appendSlice(alloc, "{\"leaf\":{\"cwd\":\"");
            try writeJsonString(alloc, buf, lf.cwd);
            try buf.appendSlice(alloc, "\"}}");
        },
        .split => |sp| {
            const hdr = try std.fmt.bufPrint(&tmp, "{{\"split\":{{\"axis\":{d},\"ratio\":{d:.6},\"a\":", .{ sp.axis, sp.ratio });
            try buf.appendSlice(alloc, hdr);
            try writeNodeJson(alloc, buf, sp.a.*);
            try buf.appendSlice(alloc, ",\"b\":");
            try writeNodeJson(alloc, buf, sp.b.*);
            try buf.appendSlice(alloc, "}}");
        },
    }
}

fn writeJsonString(alloc: std.mem.Allocator, buf: *std.ArrayListUnmanaged(u8), s: []const u8) !void {
    for (s) |ch| {
        if (ch == '"' or ch == '\\') try buf.append(alloc, '\\');
        try buf.append(alloc, ch);
    }
}

// --- Parse JSON ---

const ParseError = error{ MalformedJson, OutOfMemory };

pub fn parseJson(alloc: std.mem.Allocator, text: []const u8) ParseError!StateJson {
    var p = JsonParser{ .buf = text, .pos = 0 };
    return p.parseState(alloc) catch |e| switch (e) {
        error.OutOfMemory => return error.OutOfMemory,
        else => return error.MalformedJson,
    };
}

const JsonParser = struct {
    buf: []const u8,
    pos: usize,

    fn skipWs(self: *JsonParser) void {
        while (self.pos < self.buf.len) {
            switch (self.buf[self.pos]) {
                ' ', '\t', '\n', '\r' => self.pos += 1,
                else => break,
            }
        }
    }

    fn expect(self: *JsonParser, ch: u8) !void {
        self.skipWs();
        if (self.pos >= self.buf.len or self.buf[self.pos] != ch) return error.MalformedJson;
        self.pos += 1;
    }

    fn parseKey(self: *JsonParser) ![]const u8 {
        self.skipWs();
        if (self.pos >= self.buf.len or self.buf[self.pos] != '"') return error.MalformedJson;
        self.pos += 1;
        const start = self.pos;
        while (self.pos < self.buf.len and self.buf[self.pos] != '"') : (self.pos += 1) {}
        if (self.pos >= self.buf.len) return error.MalformedJson;
        const key = self.buf[start..self.pos];
        self.pos += 1;
        return key;
    }

    fn parseString(self: *JsonParser, alloc: std.mem.Allocator) ![]u8 {
        self.skipWs();
        if (self.pos >= self.buf.len or self.buf[self.pos] != '"') return error.MalformedJson;
        self.pos += 1;
        var out: std.ArrayListUnmanaged(u8) = .empty;
        while (self.pos < self.buf.len and self.buf[self.pos] != '"') {
            if (self.buf[self.pos] == '\\') {
                self.pos += 1;
                if (self.pos >= self.buf.len) return error.MalformedJson;
            }
            try out.append(alloc, self.buf[self.pos]);
            self.pos += 1;
        }
        if (self.pos >= self.buf.len) return error.MalformedJson;
        self.pos += 1;
        return out.toOwnedSlice(alloc);
    }

    fn parseUsize(self: *JsonParser) !usize {
        self.skipWs();
        var val: usize = 0;
        var got: bool = false;
        while (self.pos < self.buf.len and self.buf[self.pos] >= '0' and self.buf[self.pos] <= '9') {
            val = val * 10 + (self.buf[self.pos] - '0');
            self.pos += 1;
            got = true;
        }
        if (!got) return error.MalformedJson;
        return val;
    }

    fn parseFloat(self: *JsonParser) !f32 {
        self.skipWs();
        const start = self.pos;
        if (self.pos < self.buf.len and self.buf[self.pos] == '-') self.pos += 1;
        while (self.pos < self.buf.len) {
            switch (self.buf[self.pos]) {
                '0'...'9', '.', 'e', 'E', '+', '-' => self.pos += 1,
                else => break,
            }
        }
        if (self.pos == start) return error.MalformedJson;
        return std.fmt.parseFloat(f32, self.buf[start..self.pos]) catch return error.MalformedJson;
    }

    fn parseState(self: *JsonParser, alloc: std.mem.Allocator) !StateJson {
        try self.expect('{');
        var version: u32 = 0;
        var active_tab: usize = 0;
        var tabs: []TabJson = &[_]TabJson{};
        var first = true;
        while (true) {
            self.skipWs();
            if (self.pos < self.buf.len and self.buf[self.pos] == '}') {
                self.pos += 1;
                break;
            }
            if (!first) try self.expect(',');
            first = false;
            const key = try self.parseKey();
            try self.expect(':');
            if (std.mem.eql(u8, key, "version")) {
                version = @intCast(try self.parseUsize());
            } else if (std.mem.eql(u8, key, "active_tab")) {
                active_tab = try self.parseUsize();
            } else if (std.mem.eql(u8, key, "tabs")) {
                tabs = try self.parseTabs(alloc);
            } else {
                try self.skipValue();
            }
        }
        if (version != 1 or tabs.len == 0) return error.MalformedJson;
        if (active_tab >= tabs.len) active_tab = 0;
        return .{ .version = version, .active_tab = active_tab, .tabs = tabs };
    }

    fn parseTabs(self: *JsonParser, alloc: std.mem.Allocator) ![]TabJson {
        try self.expect('[');
        var list: std.ArrayListUnmanaged(TabJson) = .empty;
        var first = true;
        while (true) {
            self.skipWs();
            if (self.pos < self.buf.len and self.buf[self.pos] == ']') {
                self.pos += 1;
                break;
            }
            if (!first) try self.expect(',');
            first = false;
            try list.append(alloc, try self.parseTab(alloc));
        }
        return list.toOwnedSlice(alloc);
    }

    fn parseTab(self: *JsonParser, alloc: std.mem.Allocator) !TabJson {
        try self.expect('{');
        var root_opt: ?NodeJson = null;
        var first = true;
        while (true) {
            self.skipWs();
            if (self.pos < self.buf.len and self.buf[self.pos] == '}') {
                self.pos += 1;
                break;
            }
            if (!first) try self.expect(',');
            first = false;
            const key = try self.parseKey();
            try self.expect(':');
            if (std.mem.eql(u8, key, "root")) {
                root_opt = try self.parseNode(alloc);
            } else {
                try self.skipValue();
            }
        }
        return .{ .root = root_opt orelse return error.MalformedJson };
    }

    const NodeError = error{ MalformedJson, OutOfMemory };

    fn parseNode(self: *JsonParser, alloc: std.mem.Allocator) NodeError!NodeJson {
        try self.expect('{');
        self.skipWs();
        const key = try self.parseKey();
        try self.expect(':');
        var result: NodeJson = undefined;
        if (std.mem.eql(u8, key, "leaf")) {
            result = .{ .leaf = try self.parseLeaf(alloc) };
        } else if (std.mem.eql(u8, key, "split")) {
            result = .{ .split = try self.parseSplit(alloc) };
        } else {
            return error.MalformedJson;
        }
        try self.expect('}');
        return result;
    }

    fn parseLeaf(self: *JsonParser, alloc: std.mem.Allocator) NodeError!LeafJson {
        try self.expect('{');
        var cwd: []u8 = try alloc.dupe(u8, "");
        var first = true;
        while (true) {
            self.skipWs();
            if (self.pos < self.buf.len and self.buf[self.pos] == '}') {
                self.pos += 1;
                break;
            }
            if (!first) try self.expect(',');
            first = false;
            const key = try self.parseKey();
            try self.expect(':');
            if (std.mem.eql(u8, key, "cwd")) {
                cwd = try self.parseString(alloc);
            } else {
                try self.skipValue();
            }
        }
        return .{ .cwd = cwd };
    }

    fn parseSplit(self: *JsonParser, alloc: std.mem.Allocator) NodeError!SplitJson {
        try self.expect('{');
        var axis: u8 = 0;
        var ratio: f32 = 0.5;
        var a_opt: ?*NodeJson = null;
        var b_opt: ?*NodeJson = null;
        var first = true;
        while (true) {
            self.skipWs();
            if (self.pos < self.buf.len and self.buf[self.pos] == '}') {
                self.pos += 1;
                break;
            }
            if (!first) try self.expect(',');
            first = false;
            const key = try self.parseKey();
            try self.expect(':');
            if (std.mem.eql(u8, key, "axis")) {
                axis = @intCast(try self.parseUsize());
            } else if (std.mem.eql(u8, key, "ratio")) {
                ratio = try self.parseFloat();
            } else if (std.mem.eql(u8, key, "a")) {
                const ptr = try alloc.create(NodeJson);
                ptr.* = try self.parseNode(alloc);
                a_opt = ptr;
            } else if (std.mem.eql(u8, key, "b")) {
                const ptr = try alloc.create(NodeJson);
                ptr.* = try self.parseNode(alloc);
                b_opt = ptr;
            } else {
                try self.skipValue();
            }
        }
        return .{
            .axis = axis,
            .ratio = ratio,
            .a = a_opt orelse return error.MalformedJson,
            .b = b_opt orelse return error.MalformedJson,
        };
    }

    fn skipValue(self: *JsonParser) !void {
        self.skipWs();
        if (self.pos >= self.buf.len) return error.MalformedJson;
        switch (self.buf[self.pos]) {
            '"' => {
                self.pos += 1;
                while (self.pos < self.buf.len and self.buf[self.pos] != '"') {
                    if (self.buf[self.pos] == '\\') self.pos += 1;
                    self.pos += 1;
                }
                if (self.pos < self.buf.len) self.pos += 1;
            },
            '{' => {
                self.pos += 1;
                var depth: usize = 1;
                while (self.pos < self.buf.len and depth > 0) {
                    if (self.buf[self.pos] == '"') {
                        self.pos += 1;
                        while (self.pos < self.buf.len and self.buf[self.pos] != '"') {
                            if (self.buf[self.pos] == '\\') self.pos += 1;
                            self.pos += 1;
                        }
                        if (self.pos < self.buf.len) self.pos += 1;
                    } else {
                        if (self.buf[self.pos] == '{') depth += 1;
                        if (self.buf[self.pos] == '}') depth -= 1;
                        self.pos += 1;
                    }
                }
            },
            '[' => {
                self.pos += 1;
                var depth: usize = 1;
                while (self.pos < self.buf.len and depth > 0) {
                    if (self.buf[self.pos] == '[') depth += 1;
                    if (self.buf[self.pos] == ']') depth -= 1;
                    self.pos += 1;
                }
            },
            else => {
                while (self.pos < self.buf.len) {
                    switch (self.buf[self.pos]) {
                        ',', '}', ']' => break,
                        else => self.pos += 1,
                    }
                }
            },
        }
    }
};

// --- File I/O ---

pub fn sessionFilePath(buf: []u8) ?[:0]u8 {
    const home = c.getenv("HOME") orelse return null;
    const h = std.mem.span(home);
    return std.fmt.bufPrintZ(buf, "{s}/.config/anvil/session.json", .{h}) catch null;
}

pub fn saveToFile(alloc: std.mem.Allocator, mgr: *const SessionManager) void {
    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path = sessionFilePath(&path_buf) orelse return;

    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const aa = arena.allocator();

    const state = capture(aa, mgr) catch return;
    const json = writeJson(aa, state) catch return;

    const home_raw = c.getenv("HOME") orelse return;
    var dir_buf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = std.fmt.bufPrintZ(&dir_buf, "{s}/.config/anvil", .{std.mem.span(home_raw)}) catch return;
    _ = c.mkdir(dir, 0o755);

    const f = c.fopen(path, "w") orelse return;
    defer _ = c.fclose(f);
    _ = c.fwrite(json.ptr, 1, json.len, f);
}

pub fn loadFromFile(alloc: std.mem.Allocator) ?StateJson {
    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path = sessionFilePath(&path_buf) orelse return null;

    const f = c.fopen(path, "r") orelse return null;
    defer _ = c.fclose(f);

    _ = c.fseek(f, 0, c.SEEK_END);
    const raw_sz = c.ftell(f);
    _ = c.fseek(f, 0, c.SEEK_SET);
    if (raw_sz <= 0 or raw_sz > 1 << 20) return null;
    const sz: usize = @intCast(raw_sz);

    const buf = alloc.alloc(u8, sz) catch return null;
    defer alloc.free(buf);
    const n = c.fread(buf.ptr, 1, sz, f);
    if (n != sz) return null;

    return parseJson(alloc, buf[0..n]) catch null;
}

// --- Unit tests ---

test "serialize and deserialize round-trip" {
    const alloc = std.testing.allocator;

    const a_node = NodeJson{ .leaf = .{ .cwd = "/home/user/projects" } };
    const b_node = NodeJson{ .leaf = .{ .cwd = "/tmp" } };
    var a_owned = a_node;
    var b_owned = b_node;

    const state = StateJson{
        .version = 1,
        .active_tab = 0,
        .tabs = &[_]TabJson{
            .{ .root = .{ .split = .{
                .axis = 0,
                .ratio = 0.6,
                .a = &a_owned,
                .b = &b_owned,
            } } },
        },
    };

    const json = try writeJson(alloc, state);
    defer alloc.free(json);

    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const parsed = try parseJson(arena.allocator(), json);

    try std.testing.expectEqual(@as(u32, 1), parsed.version);
    try std.testing.expectEqual(@as(usize, 1), parsed.tabs.len);
    try std.testing.expectEqual(@as(usize, 0), parsed.active_tab);
    const root = parsed.tabs[0].root;
    try std.testing.expect(root == .split);
    try std.testing.expectEqual(@as(u8, 0), root.split.axis);
    try std.testing.expectApproxEqAbs(@as(f32, 0.6), root.split.ratio, 0.001);
    try std.testing.expect(root.split.a.* == .leaf);
    try std.testing.expectEqualStrings("/home/user/projects", root.split.a.leaf.cwd);
    try std.testing.expect(root.split.b.* == .leaf);
    try std.testing.expectEqualStrings("/tmp", root.split.b.leaf.cwd);
}

test "malformed json returns error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    try std.testing.expectError(error.MalformedJson, parseJson(arena.allocator(), "not json"));
}

test "wrong version returns error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    try std.testing.expectError(error.MalformedJson, parseJson(arena.allocator(),
        \\{"version":2,"active_tab":0,"tabs":[{"root":{"leaf":{"cwd":""}}}]}
    ));
}

test "single leaf round-trip" {
    const alloc = std.testing.allocator;
    const state = StateJson{
        .version = 1,
        .active_tab = 0,
        .tabs = &[_]TabJson{.{ .root = .{ .leaf = .{ .cwd = "/Users/test" } } }},
    };
    const json = try writeJson(alloc, state);
    defer alloc.free(json);

    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const parsed = try parseJson(arena.allocator(), json);
    try std.testing.expectEqual(@as(usize, 1), parsed.tabs.len);
    try std.testing.expect(parsed.tabs[0].root == .leaf);
    try std.testing.expectEqualStrings("/Users/test", parsed.tabs[0].root.leaf.cwd);
}
