//! Detects what kind of directory the prompt is sitting in, so the prompt can
//! adapt. Pure checks against the filesystem via std.c POSIX primitives
//! (Zig 0.16's high-level std.fs API requires an std.Io; std.c avoids that).

const std = @import("std");

pub const Lang = enum { none, zig, node, python, rust, go };

pub const Context = struct {
    in_git: bool = false,
    lang: Lang = .none,
    has_container: bool = false,
    has_k8s: bool = false,
};

/// True if `dir/name` exists.
fn exists(dir: []const u8, name: []const u8) bool {
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const path = std.fmt.bufPrintZ(&buf, "{s}/{s}", .{ dir, name }) catch return false;
    return std.c.access(path.ptr, 0) == 0; // 0 = F_OK
}

/// Inspect `dir` and classify it.
pub fn detect(dir: []const u8) Context {
    var c = Context{};
    c.in_git = exists(dir, ".git");
    if (exists(dir, "build.zig")) {
        c.lang = .zig;
    } else if (exists(dir, "package.json")) {
        c.lang = .node;
    } else if (exists(dir, "Cargo.toml")) {
        c.lang = .rust;
    } else if (exists(dir, "go.mod")) {
        c.lang = .go;
    } else if (exists(dir, "pyproject.toml") or exists(dir, "requirements.txt")) {
        c.lang = .python;
    }
    c.has_container = exists(dir, "Dockerfile") or exists(dir, "docker-compose.yml") or
        exists(dir, "compose.yaml");
    c.has_k8s = exists(dir, "kustomization.yaml") or exists(dir, "Chart.yaml") or
        exists(dir, "k8s");
    return c;
}

const testing = std.testing;

test "detect classifies a zig git repo" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    const io = testing.io;
    try tmp.dir.writeFile(io, .{ .sub_path = "build.zig", .data = "" });
    try tmp.dir.createDir(io, ".git", .default_dir);

    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    const len = try tmp.dir.realPath(io, &pbuf);
    const c = detect(pbuf[0..len]);

    try testing.expect(c.in_git);
    try testing.expectEqual(Lang.zig, c.lang);
    try testing.expect(!c.has_container);
}

test "detect finds a node app with docker" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    const io = testing.io;
    try tmp.dir.writeFile(io, .{ .sub_path = "package.json", .data = "{}" });
    try tmp.dir.writeFile(io, .{ .sub_path = "Dockerfile", .data = "" });

    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    const len = try tmp.dir.realPath(io, &pbuf);
    const c = detect(pbuf[0..len]);

    try testing.expectEqual(Lang.node, c.lang);
    try testing.expect(c.has_container);
    try testing.expect(!c.in_git);
}

test "detect on a plain directory yields all-false" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    const io = testing.io;
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    const len = try tmp.dir.realPath(io, &pbuf);
    const c = detect(pbuf[0..len]);
    try testing.expect(!c.in_git and c.lang == .none and !c.has_container and !c.has_k8s);
}
