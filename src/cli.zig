const std = @import("std");

pub const Mode = enum { run, dump, help, version, client };

pub const CliArgs = struct {
    mode: Mode = .run,
    /// Non-null for dump mode.
    dump_path: ?[]const u8 = null,
    /// Non-null when a positional path arg was given and validated.
    start_dir: ?[]const u8 = null,
    /// True when --new was passed: skip session restore and save.
    fresh: bool = false,
    /// IPC verb for client mode ("split", "tab", or "run").
    verb: []const u8 = "",
    /// Optional argument for the IPC verb (axis "h"/"v" or path).
    verb_arg: ?[]const u8 = null,
    /// For "run": every argv token after the verb (the command + its args).
    run_args: []const [*:0]const u8 = &.{},
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
        if (std.mem.eql(u8, a, "split")) {
            result.mode = .client;
            result.verb = "split";
            i += 1;
            if (i < args.len) result.verb_arg = std.mem.span(args[i]);
            return result;
        }
        if (std.mem.eql(u8, a, "tab")) {
            result.mode = .client;
            result.verb = "tab";
            i += 1;
            if (i < args.len) result.verb_arg = std.mem.span(args[i]);
            return result;
        }
        if (std.mem.eql(u8, a, "run")) {
            result.mode = .client;
            result.verb = "run";
            result.run_args = args[i + 1 ..];
            return result;
        }
        if (std.mem.eql(u8, a, "pipe")) {
            result.mode = .client;
            result.verb = "pipe";
            return result;
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
    \\Verbs (drive a running window):
    \\  split h|v         Split the focused pane horizontally or vertically
    \\  tab [path]        Open a new tab (optionally in path)
    \\  run <cmd...>      Open a new tab and run the command in it
    \\  pipe              Page stdin in a new tab (e.g. `cmd | anvil pipe`)
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

test "parse: split h → client mode, verb split, arg h" {
    const a = parse(&.{ "split", "h" });
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("split", a.verb);
    try std.testing.expectEqualStrings("h", a.verb_arg.?);
}

test "parse: split v → client mode, verb split, arg v" {
    const a = parse(&.{ "split", "v" });
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("split", a.verb);
    try std.testing.expectEqualStrings("v", a.verb_arg.?);
}

test "parse: tab /x → client mode, verb tab, arg /x" {
    const a = parse(&.{ "tab", "/x" });
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("tab", a.verb);
    try std.testing.expectEqualStrings("/x", a.verb_arg.?);
}

test "parse: bare tab → client mode, verb tab, no arg" {
    const a = parse(&.{"tab"});
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("tab", a.verb);
    try std.testing.expect(a.verb_arg == null);
}

test "parse: run echo hi → client mode, verb run, run_args captured" {
    const a = parse(&.{ "run", "echo", "hi" });
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("run", a.verb);
    try std.testing.expectEqual(@as(usize, 2), a.run_args.len);
    try std.testing.expectEqualStrings("echo", std.mem.span(a.run_args[0]));
    try std.testing.expectEqualStrings("hi", std.mem.span(a.run_args[1]));
}

test "parse: pipe → client mode, verb pipe" {
    const a = parse(&.{"pipe"});
    try std.testing.expectEqual(Mode.client, a.mode);
    try std.testing.expectEqualStrings("pipe", a.verb);
}
