const std = @import("std");

const c = @cImport({
    @cInclude("stdio.h");
    @cInclude("sys/stat.h");
    @cInclude("unistd.h");
});

/// Parse a `.git/HEAD` line into `buf`. Returns a slice into `buf`, or empty.
pub fn parseHeadLine(line: []const u8, buf: []u8) []const u8 {
    const prefix = "ref: refs/heads/";
    if (std.mem.startsWith(u8, line, prefix)) {
        const branch = line[prefix.len..];
        const m = @min(branch.len, buf.len);
        @memcpy(buf[0..m], branch[0..m]);
        return buf[0..m];
    }
    // Detached HEAD: raw sha — show first 7 hex chars.
    if (line.len >= 7) {
        var ok = true;
        for (line[0..7]) |ch| {
            if (!std.ascii.isHex(ch)) {
                ok = false;
                break;
            }
        }
        if (ok) {
            @memcpy(buf[0..7], line[0..7]);
            return buf[0..7];
        }
    }
    return "";
}

/// Parse a `current-context:` line from kubeconfig content into `buf`.
/// Returns a slice into `buf`, or empty if not found.
pub fn parseKubeCurrentContext(content: []const u8, buf: []u8) []const u8 {
    const key = "current-context:";
    var it = std.mem.splitScalar(u8, content, '\n');
    while (it.next()) |raw_line| {
        const line = std.mem.trimStart(u8, raw_line, " \t");
        if (!std.mem.startsWith(u8, line, key)) continue;
        const val = std.mem.trim(u8, line[key.len..], " \t\r'\"");
        if (val.len == 0) continue;
        const m = @min(val.len, buf.len);
        @memcpy(buf[0..m], val[0..m]);
        return buf[0..m];
    }
    return "";
}

fn readFileCapped(path: [*:0]const u8, buf: []u8) []u8 {
    const f = c.fopen(path, "rb") orelse return buf[0..0];
    defer _ = c.fclose(f);
    const n = c.fread(buf.ptr, 1, buf.len, f);
    return buf[0..n];
}

fn statExists(path_buf: []const u8) bool {
    var tmp: [std.fs.max_path_bytes + 1]u8 = undefined;
    const n = @min(path_buf.len, tmp.len - 1);
    @memcpy(tmp[0..n], path_buf[0..n]);
    tmp[n] = 0;
    var st: c.struct_stat = undefined;
    return c.stat(@ptrCast(&tmp), &st) == 0;
}

/// Walk up from `cwd` to find a `.git` entry. Returns the `.git` path into
/// `out_buf` (null-terminated), or null if not found.
fn findDotGit(cwd: []const u8, out_buf: *[std.fs.max_path_bytes + 1]u8) ?[:0]const u8 {
    var dir = cwd;
    while (true) {
        const candidate = std.fmt.bufPrint(out_buf[0 .. out_buf.len - 1], "{s}/.git", .{dir}) catch return null;
        out_buf[candidate.len] = 0;
        if (statExists(candidate)) return out_buf[0..candidate.len :0];
        const parent = std.fs.path.dirname(dir) orelse return null;
        if (std.mem.eql(u8, parent, dir)) return null;
        dir = parent;
    }
}

/// Given the focused pane's cwd, return the git branch (or short sha) into `buf`.
/// Empty if not inside a repo or on error.
pub fn gitBranch(cwd: []const u8, buf: []u8) []const u8 {
    if (cwd.len == 0) return "";
    var dotgit_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const dot_git = findDotGit(cwd, &dotgit_buf) orelse return "";

    var head_path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const head_path = std.fmt.bufPrintZ(&head_path_buf, "{s}/HEAD", .{dot_git}) catch return "";

    var raw: [256]u8 = undefined;
    const data = readFileCapped(head_path.ptr, &raw);
    if (data.len == 0) return "";
    const line = std.mem.trimEnd(u8, data, "\r\n ");
    return parseHeadLine(line, buf);
}

/// Read the kubeconfig and return current-context into `buf`.
/// Checks `$KUBECONFIG` (first path if colon-separated), then `~/.kube/config`.
pub fn kubeContext(buf: []u8) []const u8 {
    if (std.c.getenv("KUBECONFIG")) |kc| {
        const kc_str = std.mem.span(kc);
        const first = if (std.mem.indexOfScalar(u8, kc_str, ':')) |i| kc_str[0..i] else kc_str;
        if (first.len > 0) {
            var path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
            const n = @min(first.len, path_buf.len - 1);
            @memcpy(path_buf[0..n], first[0..n]);
            path_buf[n] = 0;
            if (readKubeConfig(@ptrCast(&path_buf), buf)) |ctx| return ctx;
        }
    }
    const home = std.c.getenv("HOME") orelse return "";
    var path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    const p = std.fmt.bufPrintZ(&path_buf, "{s}/.kube/config", .{std.mem.span(home)}) catch return "";
    return readKubeConfig(p.ptr, buf) orelse "";
}

fn readKubeConfig(path: [*:0]const u8, buf: []u8) ?[]const u8 {
    var raw: [65536]u8 = undefined;
    const data = readFileCapped(path, &raw);
    if (data.len == 0) return null;
    const result = parseKubeCurrentContext(data, buf);
    return if (result.len > 0) result else null;
}

/// Cached context chip. Recomputed only when the focused pane's cwd changes.
pub const Chip = struct {
    cwd_hash: u64 = 0,
    branch_buf: [64]u8 = undefined,
    branch_len: usize = 0,
    kube_buf: [64]u8 = undefined,
    kube_len: usize = 0,

    pub fn branch(self: *const Chip) []const u8 {
        return self.branch_buf[0..self.branch_len];
    }

    pub fn kube(self: *const Chip) []const u8 {
        return self.kube_buf[0..self.kube_len];
    }

    pub fn isEmpty(self: *const Chip) bool {
        return self.branch_len == 0 and self.kube_len == 0;
    }

    pub fn update(self: *Chip, cwd: []const u8) void {
        const h = std.hash.Wyhash.hash(0, cwd);
        if (self.cwd_hash != 0 and h == self.cwd_hash) return;
        self.cwd_hash = h;

        const b = gitBranch(cwd, &self.branch_buf);
        self.branch_len = b.len;

        const k = kubeContext(&self.kube_buf);
        self.kube_len = k.len;
    }
};

// --- Tests ---

test "parseHeadLine: ref branch" {
    var buf: [64]u8 = undefined;
    const result = parseHeadLine("ref: refs/heads/main", &buf);
    try std.testing.expectEqualStrings("main", result);
}

test "parseHeadLine: ref with slash in branch name" {
    var buf: [64]u8 = undefined;
    const result = parseHeadLine("ref: refs/heads/feat/my-feature", &buf);
    try std.testing.expectEqualStrings("feat/my-feature", result);
}

test "parseHeadLine: detached sha" {
    var buf: [64]u8 = undefined;
    const result = parseHeadLine("abc1234def567890abcd1234", &buf);
    try std.testing.expectEqualStrings("abc1234", result);
}

test "parseHeadLine: garbage returns empty" {
    var buf: [64]u8 = undefined;
    const result = parseHeadLine("not-a-ref", &buf);
    try std.testing.expectEqualStrings("", result);
}

test "parseHeadLine: empty returns empty" {
    var buf: [64]u8 = undefined;
    const result = parseHeadLine("", &buf);
    try std.testing.expectEqualStrings("", result);
}

test "parseKubeCurrentContext: finds current-context" {
    const yaml =
        \\apiVersion: v1
        \\clusters: []
        \\current-context: prod-us
        \\contexts: []
    ;
    var buf: [64]u8 = undefined;
    const result = parseKubeCurrentContext(yaml, &buf);
    try std.testing.expectEqualStrings("prod-us", result);
}

test "parseKubeCurrentContext: handles whitespace and quotes" {
    const yaml =
        \\current-context:  "staging-eu"
    ;
    var buf: [64]u8 = undefined;
    const result = parseKubeCurrentContext(yaml, &buf);
    try std.testing.expectEqualStrings("staging-eu", result);
}

test "parseKubeCurrentContext: missing returns empty" {
    const yaml =
        \\apiVersion: v1
        \\clusters: []
    ;
    var buf: [64]u8 = undefined;
    const result = parseKubeCurrentContext(yaml, &buf);
    try std.testing.expectEqualStrings("", result);
}

test "parseKubeCurrentContext: garbage returns empty" {
    var buf: [64]u8 = undefined;
    const result = parseKubeCurrentContext("not yaml at all %%!!", &buf);
    try std.testing.expectEqualStrings("", result);
}
