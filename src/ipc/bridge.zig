//! Typed native<->web IPC message protocol for the embedded webview.
//! JSON strings cross the bridge both directions. This module owns the whole
//! message catalog: encode (native -> web) and decode (web -> native).

const std = @import("std");

// --- web -> native -------------------------------------------------------

/// A message posted by the web surface. `invoke.id` is owned by the caller's
/// allocator — call `deinit` to free it.
pub const Inbound = union(enum) {
    ready,
    invoke: []const u8,
    dismiss,

    pub fn deinit(self: Inbound, allocator: std.mem.Allocator) void {
        switch (self) {
            .invoke => |id| allocator.free(id),
            else => {},
        }
    }
};

pub const DecodeError = error{
    InvalidJson,
    UnknownType,
    MissingField,
} || std.mem.Allocator.Error;

/// Parse a JSON message from the web surface. The returned `Inbound` owns no
/// memory except `invoke.id`, which is duped into `allocator`.
pub fn decode(allocator: std.mem.Allocator, json: []const u8) DecodeError!Inbound {
    const Wire = struct {
        type: []const u8,
        id: ?[]const u8 = null,
    };
    const parsed = std.json.parseFromSlice(Wire, allocator, json, .{
        .ignore_unknown_fields = true,
    }) catch return error.InvalidJson;
    defer parsed.deinit();

    const w = parsed.value;
    if (std.mem.eql(u8, w.type, "ready")) return .ready;
    if (std.mem.eql(u8, w.type, "dismiss")) return .dismiss;
    if (std.mem.eql(u8, w.type, "invoke")) {
        const id = w.id orelse return error.MissingField;
        return .{ .invoke = try allocator.dupe(u8, id) };
    }
    return error.UnknownType;
}

test "decode ready" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"ready\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .ready);
}

test "decode dismiss" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"dismiss\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .dismiss);
}

test "decode invoke carries the command id" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"invoke\",\"id\":\"theme.dark\"}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .invoke);
    try std.testing.expectEqualStrings("theme.dark", msg.invoke);
}

test "decode ignores unknown fields" {
    const msg = try decode(std.testing.allocator, "{\"type\":\"ready\",\"extra\":99}");
    defer msg.deinit(std.testing.allocator);
    try std.testing.expect(msg == .ready);
}

test "decode invoke without id fails" {
    try std.testing.expectError(error.MissingField, decode(std.testing.allocator, "{\"type\":\"invoke\"}"));
}

test "decode unknown type fails" {
    try std.testing.expectError(error.UnknownType, decode(std.testing.allocator, "{\"type\":\"banana\"}"));
}

test "decode malformed json fails" {
    try std.testing.expectError(error.InvalidJson, decode(std.testing.allocator, "{not json"));
}
