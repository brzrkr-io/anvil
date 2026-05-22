//! Pure helpers for interactive terminal actions: token extraction at a column,
//! line-suffix stripping, and URL-vs-path classification. No platform I/O.
//! These are extracted for unit testability; the glue lives in main.zig.

const std = @import("std");

/// Extract the whitespace-delimited token from `line` that spans column `col`.
/// `line` is a UTF-8 string; `col` is a byte-column index (0-based).
/// Returns a sub-slice of `line`, or empty if `col` is out of range or the
/// character at `col` is whitespace.
pub fn tokenAtCol(line: []const u8, col: usize) []const u8 {
    if (col >= line.len) return "";
    // If the clicked character is whitespace, there's no token here.
    if (std.ascii.isWhitespace(line[col])) return "";

    // Scan left to find the start of the token.
    var start = col;
    while (start > 0 and !std.ascii.isWhitespace(line[start - 1])) {
        start -= 1;
    }

    // Scan right to find the end of the token.
    var end = col + 1;
    while (end < line.len and !std.ascii.isWhitespace(line[end])) {
        end += 1;
    }

    return line[start..end];
}

/// Strip a trailing `:line` or `:line:col` numeric suffix from `tok`.
/// E.g. "src/main.zig:412" → "src/main.zig", "foo.zig:10:3" → "foo.zig".
/// Returns a sub-slice of `tok` (no allocation).
pub fn stripLineSuffix(tok: []const u8) []const u8 {
    // Work backwards: strip an optional `:digits` suffix up to twice.
    var s = tok;

    // Try to strip `:col` first (second suffix), then `:line` (first suffix).
    // We do this twice so we handle both `:line` and `:line:col`.
    var iterations: usize = 0;
    while (iterations < 2) : (iterations += 1) {
        // Find the last ':' in s.
        const colon = std.mem.lastIndexOfScalar(u8, s, ':') orelse break;
        if (colon == 0) break; // colon at start — no path before it
        const after = s[colon + 1 ..];
        if (after.len == 0) break; // trailing colon with nothing after
        // Verify all chars after ':' are ASCII digits.
        var all_digits = true;
        for (after) |ch| {
            if (ch < '0' or ch > '9') {
                all_digits = false;
                break;
            }
        }
        if (!all_digits) break;
        s = s[0..colon];
    }
    return s;
}

/// Classification of a terminal token for ⌘-click handling.
pub const Kind = enum { url, path, none };

/// Classify `tok` (already suffix-stripped) as a URL, a file path, or neither.
/// `cwd` is used to probe for file existence when other heuristics are
/// inconclusive. Pass an empty slice to skip the existence check.
pub fn classify(tok: []const u8, cwd: []const u8) Kind {
    if (tok.len == 0) return .none;

    // URL: starts with http:// or https://.
    if (std.mem.startsWith(u8, tok, "http://") or
        std.mem.startsWith(u8, tok, "https://")) return .url;

    // Absolute path.
    if (tok[0] == '/') return .path;

    // Relative path heuristics: contains '/' or has a '.' extension.
    const has_slash = std.mem.indexOfScalar(u8, tok, '/') != null;
    const has_ext = blk: {
        // A dot that is not the first character and has at least one char after it.
        if (std.mem.lastIndexOfScalar(u8, tok, '.')) |dot| {
            break :blk dot > 0 and dot + 1 < tok.len;
        }
        break :blk false;
    };

    if (has_slash or has_ext) return .path;

    // Last resort: does the file exist relative to cwd?
    if (cwd.len > 0 and fileExistsRelative(cwd, tok)) return .path;

    return .none;
}

/// True if `name` exists (any type) relative to `dir`. Uses POSIX `access(2)`.
fn fileExistsRelative(dir: []const u8, name: []const u8) bool {
    // Build a null-terminated path: dir + "/" + name.
    var buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const path = std.fmt.bufPrint(buf[0 .. buf.len - 1], "{s}/{s}", .{ dir, name }) catch return false;
    buf[path.len] = 0;
    const path_z: [*:0]const u8 = buf[0..path.len :0];
    return std.c.access(path_z, 0) == 0; // F_OK = 0
}

// --- Tests ------------------------------------------------------------------

const testing = std.testing;

test "tokenAtCol: basic token extraction" {
    try testing.expectEqualStrings("hello", tokenAtCol("hello world", 0));
    try testing.expectEqualStrings("hello", tokenAtCol("hello world", 3));
    try testing.expectEqualStrings("hello", tokenAtCol("hello world", 4));
    try testing.expectEqualStrings("world", tokenAtCol("hello world", 6));
    try testing.expectEqualStrings("world", tokenAtCol("hello world", 10));
}

test "tokenAtCol: whitespace at col returns empty" {
    try testing.expectEqualStrings("", tokenAtCol("hello world", 5));
}

test "tokenAtCol: col out of range returns empty" {
    try testing.expectEqualStrings("", tokenAtCol("hi", 99));
}

test "tokenAtCol: single token" {
    try testing.expectEqualStrings("src/main.zig:412", tokenAtCol("src/main.zig:412", 5));
}

test "tokenAtCol: leading whitespace" {
    try testing.expectEqualStrings("foo", tokenAtCol("  foo  bar", 3));
    try testing.expectEqualStrings("bar", tokenAtCol("  foo  bar", 7));
}

test "stripLineSuffix: strips :line" {
    try testing.expectEqualStrings("src/main.zig", stripLineSuffix("src/main.zig:412"));
}

test "stripLineSuffix: strips :line:col" {
    try testing.expectEqualStrings("src/main.zig", stripLineSuffix("src/main.zig:412:3"));
}

test "stripLineSuffix: no suffix unchanged" {
    try testing.expectEqualStrings("src/main.zig", stripLineSuffix("src/main.zig"));
}

test "stripLineSuffix: non-digit suffix unchanged" {
    try testing.expectEqualStrings("foo:bar", stripLineSuffix("foo:bar"));
}

test "stripLineSuffix: trailing colon unchanged" {
    try testing.expectEqualStrings("foo:", stripLineSuffix("foo:"));
}

test "classify: URLs" {
    try testing.expectEqual(Kind.url, classify("http://example.com", ""));
    try testing.expectEqual(Kind.url, classify("https://example.com/path", ""));
}

test "classify: absolute path" {
    try testing.expectEqual(Kind.path, classify("/usr/local/bin", ""));
}

test "classify: relative path with slash" {
    try testing.expectEqual(Kind.path, classify("src/main.zig", ""));
}

test "classify: relative path with extension" {
    try testing.expectEqual(Kind.path, classify("main.zig", ""));
}

test "classify: bare word with no heuristics" {
    try testing.expectEqual(Kind.none, classify("hello", ""));
}

test "classify: empty token" {
    try testing.expectEqual(Kind.none, classify("", ""));
}
