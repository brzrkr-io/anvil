const std = @import("std");

pub const Mode = enum { run, dump, help, version };

pub const CliArgs = struct {
    mode: Mode = .run,
    /// Non-null for dump mode.
    dump_path: ?[]const u8 = null,
    /// Non-null when a positional path arg was given and validated.
    start_dir: ?[]const u8 = null,
    /// True when --new was passed: skip session restore and save.
    fresh: bool = false,
};

/// Parse process argv (after skipping argv[0]).
/// `args` is a slice of null-terminated strings.
pub fn parse(args: []const [*:0]const u8) CliArgs {
    var result = CliArgs{};
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const a = std.mem.span(args[i]);
        if (std.mem.eql(u8, a, "--help") or std.mem.eql(u8, a, "-h")) {
            result.mode = .help;
            return result;
        }
        if (std.mem.eql(u8, a, "--version") or std.mem.eql(u8, a, "-v")) {
            result.mode = .version;
            return result;
        }
        if (std.mem.eql(u8, a, "--dump")) {
            i += 1;
            if (i < args.len) {
                result.mode = .dump;
                result.dump_path = std.mem.span(args[i]);
            }
            return result;
        }
        if (std.mem.eql(u8, a, "--new")) {
            result.fresh = true;
            continue;
        }
        if (!std.mem.startsWith(u8, a, "-")) {
            result.start_dir = a;
        }
    }
    return result;
}

pub const version_string = "0.1.0";

pub const help_text =
    \\Usage: anvil [options] [path]
    \\
    \\  path              Start the shell in this directory (default: $HOME or cwd)
    \\
    \\Options:
    \\  --help, -h        Show this help and exit
    \\  --version, -v     Print version and exit
    \\  --dump <path>     Headless render to PNG and exit
    \\  --new             Open a fresh window (skip session restore/save)
    \\
;

test "parse: no args → run mode" {
    const a = parse(&.{});
    try std.testing.expectEqual(Mode.run, a.mode);
    try std.testing.expect(a.start_dir == null);
    try std.testing.expect(a.dump_path == null);
}

test "parse: --help" {
    const a = parse(&.{"--help"});
    try std.testing.expectEqual(Mode.help, a.mode);
}

test "parse: -h" {
    const a = parse(&.{"-h"});
    try std.testing.expectEqual(Mode.help, a.mode);
}

test "parse: --version" {
    const a = parse(&.{"--version"});
    try std.testing.expectEqual(Mode.version, a.mode);
}

test "parse: --dump sets dump mode and path" {
    const a = parse(&.{ "--dump", "/tmp/x.png" });
    try std.testing.expectEqual(Mode.dump, a.mode);
    try std.testing.expectEqualStrings("/tmp/x.png", a.dump_path.?);
}

test "parse: positional path becomes start_dir" {
    const a = parse(&.{"/Users/me/projects"});
    try std.testing.expectEqual(Mode.run, a.mode);
    try std.testing.expectEqualStrings("/Users/me/projects", a.start_dir.?);
}

test "parse: --dump takes priority over positional" {
    const a = parse(&.{ "--dump", "/tmp/out.png", "/some/dir" });
    try std.testing.expectEqual(Mode.dump, a.mode);
    try std.testing.expectEqualStrings("/tmp/out.png", a.dump_path.?);
}

test "parse: positional before flags" {
    const a = parse(&.{ "/my/dir", "--version" });
    try std.testing.expectEqual(Mode.version, a.mode);
}

test "parse: --new sets fresh" {
    const a = parse(&.{"--new"});
    try std.testing.expectEqual(Mode.run, a.mode);
    try std.testing.expect(a.fresh);
    try std.testing.expect(a.start_dir == null);
}

test "parse: --new with path sets fresh and start_dir" {
    const a = parse(&.{ "--new", "/my/dir" });
    try std.testing.expectEqual(Mode.run, a.mode);
    try std.testing.expect(a.fresh);
    try std.testing.expectEqualStrings("/my/dir", a.start_dir.?);
}
