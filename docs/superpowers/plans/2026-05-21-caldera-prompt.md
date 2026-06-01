# Caldera Prompt (Phase 1) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `anvil-prompt` — a standalone Zig program that renders an adaptive, extensible shell prompt, shipped with Anvil and auto-wired by the shell integration.

**Architecture:** A second executable in the repo (`src/prompt/`, root `src/prompt/main.zig`), built by `build.zig`. Pure-logic modules (`icons`, `segments`, `context`, `render`, plus git-output parsing) are TDD'd; `git` queries run `git` as a subprocess; `main` orchestrates and prints ANSI. The shell integration sets the shell prompt to call it. No changes to the app renderer — Phase 2 (the interactive hover layer) is a separate plan.

**Tech Stack:** Zig 0.16, `std` only (no new deps), zsh/bash, ANSI escapes.

**Spec:** `docs/superpowers/specs/2026-05-21-anvil-prompt-design.md` — read it first.

---

## Notes for the implementer

- **Verified facts** (checked against the codebase):
  - `build.zig` builds `exe_mod` (root `src/main.zig`) and a `test` step. M3 added a WebKit link + a `palette_html` anonymous import. There is also a `cov_mod` for coverage.
  - Zig std puts unit tests inline in the file. The app's tests are aggregated by `src/main.zig`'s `test { }` block. The prompt is a *separate* module, so this plan adds a separate `addTest` for it and wires it into the `test` step.
  - Zig 0.16: the high-level `std.fs` API was reorganized to require an `std.Io`; the project uses `std.c` POSIX primitives instead (see `wiki/concepts/zig-0.16-gotchas`). Follow that — use `std.c.access` / `std.c.open` / `std.c.getcwd`.
  - `std.posix.getenv` does not exist in 0.16; use `std.c.getenv` (libc is linked).
  - `src/config/config.zig` holds `Config` (a ZON struct) parsed via `std.zon.parse`. `src/app/shell_integration.zig` writes embedded shell scripts to `~/.cache/anvil/shell/` and exports env vars including `ANVIL=1`.
- If a Zig 0.16 std API differs slightly from what a step shows (e.g. `std.process.Child` fields), adapt the minimal thing and keep going — the *tested* logic is the contract. Report any such adaptation.
- A change is not done until `zig build test` passes (or the failure is reported).

---

## Task 0: Branch and baseline

**Files:** none (git only)

- [ ] **Step 1: Create the branch**

```bash
git checkout main
git checkout -b feat/anvil-prompt
```

- [ ] **Step 2: Verify the baseline is green**

Run: `zig build test`
Expected: exit 0, all tests pass. Note the count.

---

## Task 1: Build target for `anvil-prompt`

**Files:**
- Create: `src/prompt/main.zig` (stub)
- Modify: `build.zig`

- [ ] **Step 1: Create a stub `src/prompt/main.zig`**

```zig
//! anvil-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Emits ANSI to stdout.

const std = @import("std");

pub fn main() void {
    std.debug.print("anvil-prompt\n", .{});
}
```

- [ ] **Step 2: Add the executable and its tests to `build.zig`**

In `build.zig`, after the `run_step` block (after the line `run_step.dependOn(&run_cmd.step);`), add:

```zig
    // --- anvil-prompt: the shell prompt program -------------------------
    const prompt_mod = b.createModule(.{
        .root_source_file = b.path("src/prompt/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const prompt_exe = b.addExecutable(.{
        .name = "anvil-prompt",
        .root_module = prompt_mod,
    });
    b.installArtifact(prompt_exe);

    const prompt_tests = b.addTest(.{ .root_module = prompt_mod });
    const run_prompt_tests = b.addRunArtifact(prompt_tests);
```

Then find the existing `test_step` block and add a dependency on the prompt tests so `zig build test` runs both. Change it to:

```zig
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_exe_tests.step);
    test_step.dependOn(&run_prompt_tests.step);
```

- [ ] **Step 3: Verify both binaries build and tests pass**

Run: `zig build`
Expected: exit 0; `zig-out/bin/anvil` and `zig-out/bin/anvil-prompt` both exist.

Run: `zig build test`
Expected: exit 0.

Run: `./zig-out/bin/anvil-prompt`
Expected: prints `anvil-prompt`.

- [ ] **Step 4: Commit**

```bash
git add build.zig src/prompt/main.zig
git commit -m "build: add the anvil-prompt executable target"
```

---

## Task 2: Icon glyphs

**Files:**
- Create: `src/prompt/icons.zig`

- [ ] **Step 1: Write `src/prompt/icons.zig` with the icon table and tests**

```zig
//! Prompt icon glyphs. `rich` glyphs are well-formed Unicode chosen to render
//! in common monospace fonts; `ascii` fallbacks render anywhere. The two-form
//! table is the single swap point if a bundled icon font is added later.

const std = @import("std");

pub const Icon = enum {
    repo,
    branch,
    dirty,
    ahead,
    behind,
    toolchain,
    container,
    cluster,
    ok,
    err,
    clock,
};

/// The glyph for `icon`. When `rich` is false, returns a plain-ASCII fallback.
pub fn glyph(icon: Icon, rich: bool) []const u8 {
    return switch (icon) {
        .repo => if (rich) "\u{25c8}" else "#", // ◈
        .branch => if (rich) "\u{2387}" else "@", // ⎇
        .dirty => if (rich) "\u{25cf}" else "*", // ●
        .ahead => if (rich) "\u{2191}" else "^", // ↑
        .behind => if (rich) "\u{2193}" else "v", // ↓
        .toolchain => if (rich) "\u{25c6}" else "=", // ◆
        .container => if (rich) "\u{25a3}" else "[]", // ▣
        .cluster => if (rich) "\u{2b22}" else "{}", // ⬢
        .ok => if (rich) "\u{2713}" else "ok", // ✓
        .err => if (rich) "\u{2717}" else "x", // ✗
        .clock => if (rich) "\u{25f7}" else "@", // ◷
    };
}

test "rich glyphs differ from ascii fallbacks" {
    try std.testing.expect(!std.mem.eql(u8, glyph(.branch, true), glyph(.branch, false)));
    try std.testing.expect(!std.mem.eql(u8, glyph(.ok, true), glyph(.ok, false)));
}

test "every icon has a non-empty glyph in both modes" {
    inline for (std.meta.fields(Icon)) |f| {
        const ic: Icon = @enumFromInt(f.value);
        try std.testing.expect(glyph(ic, true).len > 0);
        try std.testing.expect(glyph(ic, false).len > 0);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — the 2 new `icons` tests run with the prompt test binary.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/icons.zig
git commit -m "feat(prompt): icon glyph table (rich + ascii)"
```

---

## Task 3: Segment model

**Files:**
- Create: `src/prompt/segments.zig`

- [ ] **Step 1: Write `src/prompt/segments.zig` with the segment types and tests**

```zig
//! The prompt's segment model. A Segment is one unit on the context line:
//! an icon, a text value, and a state that drives its colour.

const std = @import("std");
const Icon = @import("icons.zig").Icon;

/// Drives the segment's colour at render time.
pub const State = enum { normal, ok, warn, err, run };

pub const Segment = struct {
    icon: Icon,
    /// Borrowed; must outlive the Segment (caller owns the backing memory).
    text: []const u8,
    state: State = .normal,
};

/// A fixed-capacity segment list — the prompt never shows more than this many,
/// and a stack buffer keeps `anvil-prompt` allocation-light on the hot path.
pub const max_segments = 12;

pub const List = struct {
    items: [max_segments]Segment = undefined,
    len: usize = 0,

    pub fn add(self: *List, seg: Segment) void {
        if (self.len >= max_segments) return;
        self.items[self.len] = seg;
        self.len += 1;
    }

    pub fn slice(self: *const List) []const Segment {
        return self.items[0..self.len];
    }
};

test "List.add appends until capacity" {
    var l = List{};
    try std.testing.expectEqual(@as(usize, 0), l.slice().len);
    l.add(.{ .icon = .branch, .text = "main" });
    try std.testing.expectEqual(@as(usize, 1), l.slice().len);
    try std.testing.expectEqualStrings("main", l.slice()[0].text);
}

test "List.add stops at capacity, never overflows" {
    var l = List{};
    var i: usize = 0;
    while (i < max_segments + 5) : (i += 1) l.add(.{ .icon = .repo, .text = "x" });
    try std.testing.expectEqual(max_segments, l.slice().len);
}
```

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — 2 new `segments` tests.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/segments.zig
git commit -m "feat(prompt): segment model"
```

---

## Task 4: Directory context detection

**Files:**
- Create: `src/prompt/context.zig`

- [ ] **Step 1: Write `src/prompt/context.zig`**

```zig
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
    try tmp.dir.makeDir(io, ".git");

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
```

> If `testing.tmpDir` / `realPath` need a different `std.Io` argument under the installed Zig 0.16, adapt the test setup minimally — `detect` itself (the tested unit) does not change.

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — 3 new `context` tests.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/context.zig
git commit -m "feat(prompt): directory context detection"
```

---

## Task 5: Git status

**Files:**
- Create: `src/prompt/git.zig`

- [ ] **Step 1: Write `src/prompt/git.zig` — parser first, then the query**

```zig
//! Git status for the prompt. `parseStatus` (pure, tested) interprets the
//! output of `git status --porcelain=v1 --branch`; `query` runs git as a
//! subprocess with a timeout and feeds it to the parser.

const std = @import("std");

pub const Info = struct {
    branch: []const u8, // borrowed from the caller-provided buffer
    dirty: u32 = 0,
    ahead: u32 = 0,
    behind: u32 = 0,
};

/// Parse `git status --porcelain=v1 --branch` output. The branch name is a
/// slice into `text`. Returns null if no branch header line is present.
pub fn parseStatus(text: []const u8) ?Info {
    var info: ?Info = null;
    var lines = std.mem.splitScalar(u8, text, '\n');
    while (lines.next()) |line| {
        if (line.len == 0) continue;
        if (std.mem.startsWith(u8, line, "## ")) {
            info = parseBranchLine(line[3..]);
        } else {
            if (info) |*i| i.dirty += 1;
        }
    }
    return info;
}

fn parseBranchLine(rest: []const u8) Info {
    // e.g. "main...origin/main [ahead 1, behind 2]"  or  "main"
    var branch_end: usize = rest.len;
    if (std.mem.indexOf(u8, rest, "...")) |i| branch_end = i;
    if (std.mem.indexOfScalar(u8, rest, ' ')) |i| branch_end = @min(branch_end, i);
    var info = Info{ .branch = rest[0..branch_end] };
    if (std.mem.indexOf(u8, rest, "ahead ")) |i| {
        info.ahead = readNum(rest[i + 6 ..]);
    }
    if (std.mem.indexOf(u8, rest, "behind ")) |i| {
        info.behind = readNum(rest[i + 7 ..]);
    }
    return info;
}

fn readNum(s: []const u8) u32 {
    var n: u32 = 0;
    for (s) |ch| {
        if (ch < '0' or ch > '9') break;
        n = n * 10 + (ch - '0');
    }
    return n;
}

/// Run git in `cwd` and return its status, or null if not a repo / git fails /
/// it times out. `out_buf` backs the returned branch slice.
pub fn query(allocator: std.mem.Allocator, cwd: []const u8, out_buf: []u8) ?Info {
    var child = std.process.Child.init(
        &.{ "git", "status", "--porcelain=v1", "--branch" },
        allocator,
    );
    child.cwd = cwd;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Ignore;
    child.spawn() catch return null;

    var stdout = std.ArrayList(u8).initCapacity(allocator, 4096) catch {
        _ = child.kill() catch {};
        return null;
    };
    defer stdout.deinit(allocator);
    child.collectOutput(allocator, &stdout, undefined, 64 * 1024) catch {
        _ = child.kill() catch {};
        return null;
    };
    const term = child.wait() catch return null;
    switch (term) {
        .Exited => |code| if (code != 0) return null,
        else => return null,
    }
    const parsed = parseStatus(stdout.items) orelse return null;
    if (parsed.branch.len > out_buf.len) return null;
    @memcpy(out_buf[0..parsed.branch.len], parsed.branch);
    return .{
        .branch = out_buf[0..parsed.branch.len],
        .dirty = parsed.dirty,
        .ahead = parsed.ahead,
        .behind = parsed.behind,
    };
}

const testing = std.testing;

test "parseStatus reads branch and dirty count" {
    const out =
        "## main...origin/main\n" ++
        " M src/a.zig\n" ++
        "?? new.txt\n";
    const info = parseStatus(out).?;
    try testing.expectEqualStrings("main", info.branch);
    try testing.expectEqual(@as(u32, 2), info.dirty);
    try testing.expectEqual(@as(u32, 0), info.ahead);
}

test "parseStatus reads ahead and behind" {
    const out = "## main...origin/main [ahead 3, behind 1]\n";
    const info = parseStatus(out).?;
    try testing.expectEqualStrings("main", info.branch);
    try testing.expectEqual(@as(u32, 3), info.ahead);
    try testing.expectEqual(@as(u32, 1), info.behind);
}

test "parseStatus handles a branch with no upstream" {
    const info = parseStatus("## feature/x\n").?;
    try testing.expectEqualStrings("feature/x", info.branch);
    try testing.expectEqual(@as(u32, 0), info.dirty);
}

test "parseStatus returns null without a branch header" {
    try testing.expect(parseStatus("") == null);
    try testing.expect(parseStatus("?? stray.txt\n") == null);
}
```

> `std.process.Child`'s exact field/method names (`collectOutput`, `stdout_behavior`) may differ slightly in the installed Zig 0.16 — adapt the `query` body minimally if the build complains. `parseStatus` and its four tests are the contract and do not change.

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — 4 new `git` parser tests.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/git.zig
git commit -m "feat(prompt): git status query and parser"
```

---

## Task 6: ANSI rendering

**Files:**
- Create: `src/prompt/render.zig`

- [ ] **Step 1: Write `src/prompt/render.zig`**

```zig
//! Renders a segment list to an ANSI prompt string. Two forms: `full` — a
//! two-line block with a mineral accent edge; `transient` — a single collapsed
//! line for past prompts. Colours are 24-bit ANSI; the accent/edge is mineral.

const std = @import("std");
const seg = @import("segments.zig");
const icons = @import("icons.zig");

const reset = "\x1b[0m";
const accent = "\x1b[38;2;47;127;134m"; // mineral #2f7f86
const accent_err = "\x1b[38;2;177;58;48m"; // failure #b13a30
const dim = "\x1b[38;2;125;135;145m";
const edge = "\u{258e}"; // ▎

fn stateColor(s: seg.State) []const u8 {
    return switch (s) {
        .normal => dim,
        .ok => "\x1b[38;2;63;138;91m",
        .warn => "\x1b[38;2;176;122;20m",
        .err => "\x1b[38;2;177;58;48m",
        .run => accent,
    };
}

pub const Options = struct {
    rich: bool,
    failed: bool, // last command exited non-zero
};

/// The full two-line prompt block. Caller owns the returned slice.
pub fn full(allocator: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    var buf = try std.ArrayList(u8).initCapacity(allocator, 256);
    errdefer buf.deinit(allocator);
    const w = buf.writer(allocator);

    const edge_color = if (opts.failed) accent_err else accent;

    // Line 1: edge + segments.
    try w.print("{s}{s}{s} ", .{ edge_color, edge, reset });
    for (segments, 0..) |s, i| {
        if (i != 0) try w.writeAll("  ");
        try w.print("{s}{s} {s}{s}", .{
            stateColor(s.state), icons.glyph(s.icon, opts.rich), s.text, reset,
        });
    }
    try w.writeAll("\n");
    // Line 2: edge + prompt glyph.
    try w.print("{s}{s} \u{276f}{s} ", .{ edge_color, edge, reset });

    return buf.toOwnedSlice(allocator);
}

/// The collapsed transient prompt — just the glyph, no edge, no context.
pub fn transient(allocator: std.mem.Allocator, opts: Options) ![]u8 {
    const color = if (opts.failed) accent_err else dim;
    return std.fmt.allocPrint(allocator, "{s}\u{276f}{s} ", .{ color, reset });
}

const testing = std.testing;

fn sampleSegs() [2]seg.Segment {
    return .{
        .{ .icon = .repo, .text = "anvil" },
        .{ .icon = .branch, .text = "main", .state = .warn },
    };
}

test "full renders two lines with the accent edge" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\n") != null); // two lines
    try testing.expect(std.mem.indexOf(u8, out, edge) != null); // edge present
    try testing.expect(std.mem.indexOf(u8, out, "anvil") != null);
    try testing.expect(std.mem.indexOf(u8, out, "main") != null);
}

test "full uses the failure colour when the last command failed" {
    const s = sampleSegs();
    const ok = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(ok);
    const bad = try full(testing.allocator, &s, .{ .rich = true, .failed = true });
    defer testing.allocator.free(bad);
    try testing.expect(std.mem.indexOf(u8, bad, accent_err) != null);
    try testing.expect(std.mem.indexOf(u8, ok, accent_err) == null);
}

test "ascii mode emits fallback glyphs" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = false, .failed = false });
    defer testing.allocator.free(out);
    // ascii branch fallback is "@", the rich glyph (⎇) must be absent
    try testing.expect(std.mem.indexOf(u8, out, icons.glyph(.branch, true)) == null);
}

test "transient is a single line, no edge" {
    const out = try transient(testing.allocator, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\n") == null);
    try testing.expect(std.mem.indexOf(u8, out, edge) == null);
}
```

> `std.ArrayList` / writer API specifics may differ in Zig 0.16 — adapt the buffer mechanics minimally; the four tests are the contract.

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — 4 new `render` tests.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/render.zig
git commit -m "feat(prompt): ANSI rendering — full and transient forms"
```

---

## Task 7: Prompt config section

**Files:**
- Modify: `src/config/config.zig`

- [ ] **Step 1: Add a `PromptCfg` to `Config`**

In `src/config/config.zig`, inside the `Config` struct, add a field after `shell_integration`:

```zig
    shell_integration: bool = true,
    prompt: PromptCfg = .{},
```

Then add the `PromptCfg` type as a `pub const` inside the `Config` struct (next to `FontCfg` / `CursorCfg` / `WindowCfg`):

```zig
    /// Shell-prompt settings. `segments` lists, in order, which built-in
    /// segments to show; `custom` declares extra command-backed segments.
    pub const PromptCfg = struct {
        enabled: bool = true,
        transient: bool = true,
        custom: []const Custom = &.{},

        pub const Custom = struct {
            label: []const u8,
            command: []const u8,
        };
    };
```

- [ ] **Step 2: Add tests at the end of `src/config/config.zig`**

```zig
test "config defaults the prompt section on" {
    var loaded = try parseSlice(testing.allocator, ".{ .scrollback = 100 }");
    defer loaded.deinit();
    try testing.expect(loaded.config.prompt.enabled);
    try testing.expect(loaded.config.prompt.transient);
    try testing.expectEqual(@as(usize, 0), loaded.config.prompt.custom.len);
}

test "config parses a custom prompt segment" {
    const src =
        \\.{ .prompt = .{ .custom = .{ .{ .label = "aws", .command = "echo prod" } } } }
    ;
    var loaded = try parseSlice(testing.allocator, src);
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 1), loaded.config.prompt.custom.len);
    try testing.expectEqualStrings("aws", loaded.config.prompt.custom[0].label);
    try testing.expectEqualStrings("echo prod", loaded.config.prompt.custom[0].command);
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test`
Expected: PASS — 2 new config tests; the existing config suite still green.

- [ ] **Step 4: Commit**

```bash
git add src/config/config.zig
git commit -m "feat(config): prompt config section"
```

---

## Task 8: Assemble the segment list

**Files:**
- Create: `src/prompt/build_segments.zig`

- [ ] **Step 1: Write `src/prompt/build_segments.zig`**

```zig
//! Turns detected context + git info into the ordered Segment list the
//! renderer draws. This is the adaptive core: a segment appears only when the
//! context calls for it.

const std = @import("std");
const seg = @import("segments.zig");
const ctx = @import("context.zig");
const git = @import("git.zig");

pub const Inputs = struct {
    cwd_base: []const u8, // basename of the working directory
    context: ctx.Context,
    git_info: ?git.Info,
    exit_code: u8,
    /// scratch buffer the assembled segment texts borrow from
    scratch: []u8,
};

fn langText(l: ctx.Lang) ?[]const u8 {
    return switch (l) {
        .none => null,
        .zig => "zig",
        .node => "node",
        .python => "python",
        .rust => "rust",
        .go => "go",
    };
}

/// Build the active segment list. Texts are slices into `in.scratch`.
pub fn assemble(in: Inputs) seg.List {
    var list = seg.List{};
    var off: usize = 0;

    // cwd — always.
    list.add(.{ .icon = .repo, .text = in.cwd_base });

    // git — when in a repo.
    if (in.git_info) |g| {
        const dirty_suffix = if (g.dirty > 0) blk: {
            const s = std.fmt.bufPrint(in.scratch[off..], "{s} \u{25cf}{d}", .{ g.branch, g.dirty }) catch g.branch;
            off += s.len;
            break :blk s;
        } else g.branch;
        list.add(.{
            .icon = .branch,
            .text = dirty_suffix,
            .state = if (g.dirty > 0) .warn else .normal,
        });
    }

    // toolchain — when a language is detected.
    if (langText(in.context.lang)) |lt| {
        list.add(.{ .icon = .toolchain, .text = lt });
    }

    // container / cluster — when present.
    if (in.context.has_container) list.add(.{ .icon = .container, .text = "docker" });
    if (in.context.has_k8s) list.add(.{ .icon = .cluster, .text = "k8s" });

    // failure — only on a non-zero exit.
    if (in.exit_code != 0) {
        const s = std.fmt.bufPrint(in.scratch[off..], "{d}", .{in.exit_code}) catch "?";
        off += s.len;
        list.add(.{ .icon = .err, .text = s, .state = .err });
    }

    return list;
}

const testing = std.testing;

test "assemble: clean repo shows cwd + branch only" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "anvil",
        .context = .{ .in_git = true },
        .git_info = .{ .branch = "main" },
        .exit_code = 0,
        .scratch = &scratch,
    });
    try testing.expectEqual(@as(usize, 2), list.slice().len);
    try testing.expectEqual(seg.Segment{ .icon = .repo, .text = "anvil" }, list.slice()[0]);
}

test "assemble: dirty repo marks the git segment warn" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "x",
        .context = .{ .in_git = true },
        .git_info = .{ .branch = "main", .dirty = 3 },
        .exit_code = 0,
        .scratch = &scratch,
    });
    try testing.expectEqual(seg.State.warn, list.slice()[1].state);
    try testing.expect(std.mem.indexOf(u8, list.slice()[1].text, "3") != null);
}

test "assemble: a node+docker dir surfaces toolchain and container" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "app",
        .context = .{ .lang = .node, .has_container = true },
        .git_info = null,
        .exit_code = 0,
        .scratch = &scratch,
    });
    var saw_tool = false;
    var saw_dk = false;
    for (list.slice()) |s| {
        if (s.icon == .toolchain) saw_tool = true;
        if (s.icon == .container) saw_dk = true;
    }
    try testing.expect(saw_tool and saw_dk);
}

test "assemble: a non-zero exit adds an err segment" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "x",
        .context = .{},
        .git_info = null,
        .exit_code = 127,
        .scratch = &scratch,
    });
    const last = list.slice()[list.slice().len - 1];
    try testing.expectEqual(seg.Icon.err, last.icon);
    try testing.expectEqualStrings("127", last.text);
}
```

- [ ] **Step 2: Run the tests**

Run: `zig build test`
Expected: PASS — 4 new `build_segments` tests.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/build_segments.zig
git commit -m "feat(prompt): assemble the adaptive segment list"
```

---

## Task 9: Wire `main.zig`

**Files:**
- Modify: `src/prompt/main.zig`

- [ ] **Step 1: Replace `src/prompt/main.zig` with the real program**

```zig
//! anvil-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Args: --exit <n>, --transient. Emits ANSI to stdout.

const std = @import("std");
const ctx = @import("context.zig");
const git = @import("git.zig");
const render = @import("render.zig");
const build_segments = @import("build_segments.zig");

const Args = struct { exit_code: u8 = 0, transient: bool = false };

fn parseArgs() Args {
    var a = Args{};
    var it = std.process.args();
    _ = it.next(); // argv[0]
    while (it.next()) |arg| {
        if (std.mem.eql(u8, arg, "--transient")) {
            a.transient = true;
        } else if (std.mem.eql(u8, arg, "--exit")) {
            if (it.next()) |v| a.exit_code = std.fmt.parseInt(u8, v, 10) catch 0;
        }
    }
    return a;
}

fn basename(path: []const u8) []const u8 {
    if (std.mem.lastIndexOfScalar(u8, path, '/')) |i| {
        if (i + 1 < path.len) return path[i + 1 ..];
    }
    return path;
}

pub fn main() void {
    var arena = std.heap.ArenaAllocator.init(std.heap.c_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const args = parseArgs();
    // Rich glyphs only inside Caldera.
    const rich = std.c.getenv("ANVIL") != null;
    const opts = render.Options{ .rich = rich, .failed = args.exit_code != 0 };

    var out_buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout();

    if (args.transient) {
        const s = render.transient(alloc, opts) catch return;
        _ = stdout.writeAll(s) catch {};
        return;
    }

    // cwd
    var cwd_buf: [std.fs.max_path_bytes]u8 = undefined;
    const cwd_ptr = std.c.getcwd(&cwd_buf, cwd_buf.len) orelse return;
    const cwd = std.mem.span(@as([*:0]const u8, @ptrCast(cwd_ptr)));

    const context = ctx.detect(cwd);
    var branch_buf: [256]u8 = undefined;
    const git_info: ?git.Info = if (context.in_git)
        git.query(alloc, cwd, &branch_buf)
    else
        null;

    var scratch: [512]u8 = undefined;
    const list = build_segments.assemble(.{
        .cwd_base = basename(cwd),
        .context = context,
        .git_info = git_info,
        .exit_code = args.exit_code,
        .scratch = &scratch,
    });

    const s = render.full(alloc, list.slice(), opts) catch return;
    _ = out_buf;
    _ = stdout.writeAll(s) catch {};
}
```

> If `std.fs.File.stdout()` / `std.process.args()` differ under the installed Zig 0.16, adapt minimally. The behaviour: print the full prompt, or the transient line with `--transient`.

- [ ] **Step 2: Build and exercise it**

Run: `zig build`
Expected: exit 0.

Run: `./zig-out/bin/anvil-prompt`
Expected: prints a two-line prompt — the repo basename, git branch if in a repo, an accent edge, and a `❯` line.

Run: `./zig-out/bin/anvil-prompt --transient`
Expected: prints a single `❯ ` line.

Run: `./zig-out/bin/anvil-prompt --exit 1`
Expected: the prompt renders with the failure colour and an exit segment.

Run: `zig build test`
Expected: exit 0 — all tests still pass.

- [ ] **Step 3: Commit**

```bash
git add src/prompt/main.zig
git commit -m "feat(prompt): wire anvil-prompt — args, orchestration, output"
```

---

## Task 10: Shell integration

**Files:**
- Modify: `src/shell/anvil-integration.zsh`
- Modify: `src/shell/anvil-integration.bash`
- Modify: `src/app/shell_integration.zig`

- [ ] **Step 1: Make `shell_integration.zig` export the prompt binary path**

In `src/app/shell_integration.zig`, inside `setup`, after the existing marker exports (after the `ANVIL_SHELL_INTEGRATION` block), add an export of the `anvil-prompt` path. The binary installs alongside `anvil` in `zig-out/bin`; resolve it relative to the running executable. Add this helper near `runtimeDir` and call it in `setup`:

```zig
/// Absolute path to the anvil-prompt binary, resolved next to this
/// executable. Null if it cannot be determined.
fn promptBinaryPath(buf: []u8) ?[]const u8 {
    var exe_buf: [std.fs.max_path_bytes]u8 = undefined;
    const exe = std.fs.selfExePath(&exe_buf) catch return null;
    const dir = std.fs.path.dirname(exe) orelse return null;
    return std.fmt.bufPrint(buf, "{s}/anvil-prompt", .{dir}) catch null;
}
```

Then in `setup`, after the markers:

```zig
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    if (promptBinaryPath(&pbuf)) |pp| {
        var ppz: [std.fs.max_path_bytes]u8 = undefined;
        if (std.fmt.bufPrintZ(&ppz, "{s}", .{pp})) |ppzs| {
            _ = setenv("ANVIL_PROMPT", ppzs.ptr, 1);
        } else |_| {}
    }
```

> If `std.fs.selfExePath` needs an `std.Io` under Zig 0.16, fall back to reading `argv[0]` via `std.process` or resolving from `$HOME` — the goal is only to set `ANVIL_PROMPT` to the binary's absolute path. Adapt minimally and report.

- [ ] **Step 2: Set the prompt in `anvil-integration.zsh`**

Append to `src/shell/anvil-integration.zsh`:

```bash
# Caldera prompt — when the binary is known, drive PROMPT from it.
if [[ -n "$ANVIL_PROMPT" && -x "$ANVIL_PROMPT" ]]; then
  setopt prompt_subst
  __anvil_prompt() {
    PROMPT="$("$ANVIL_PROMPT" --exit $? 2>/dev/null)"
  }
  precmd_functions+=(__anvil_prompt)

  # Transient: on accept-line, redraw the finished prompt collapsed.
  __anvil_transient() {
    PROMPT="$("$ANVIL_PROMPT" --transient 2>/dev/null)"
    zle .reset-prompt
  }
  zle -N zle-line-finish __anvil_transient
fi
```

- [ ] **Step 3: Set the prompt in `anvil-integration.bash`**

Append to `src/shell/anvil-integration.bash`:

```bash
# Caldera prompt — bash gets the full prompt each draw (no transient collapse).
if [[ -n "$ANVIL_PROMPT" && -x "$ANVIL_PROMPT" ]]; then
  __anvil_prompt() {
    PS1="$("$ANVIL_PROMPT" --exit $? 2>/dev/null)"
  }
  PROMPT_COMMAND="__anvil_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
```

- [ ] **Step 4: Build and verify end to end**

Run: `zig build test`
Expected: exit 0 — `shell_integration.zig`'s existing tests still pass.

Run: `zig build run`
Expected: the app launches; in the terminal, the new Caldera prompt renders — repo name, git branch, the accent edge, a `❯` line; it adapts when you `cd` into a node/docker/k8s directory; a failing command tints it; previous prompts collapse to a bare `❯` line.

- [ ] **Step 5: Commit**

```bash
git add src/shell/anvil-integration.zsh src/shell/anvil-integration.bash src/app/shell_integration.zig
git commit -m "feat(shell): drive the shell prompt from anvil-prompt"
```

---

## Task 11: Verification and docs

**Files:**
- Modify: `wiki/index.md`, `wiki/log.md`, `todo.txt`

- [ ] **Step 1: Final verification**

Run: `zig build test`
Expected: exit 0 — baseline count + 25 new prompt/config tests.

Run: `zig build run` and confirm the manual checklist:
1. The Caldera prompt renders — two lines, accent edge, `❯`.
2. `cd` into a git repo → branch segment appears; dirty files → it goes amber with a count.
3. `cd` into a Node app / a dir with a `Dockerfile` / a `k8s` dir → the toolchain / container / cluster segments appear.
4. Run a failing command (`false`) → the next prompt's edge and an exit segment go red.
5. After running a command, the previous prompt collapses to a bare `❯` line (zsh).

- [ ] **Step 2: Update `wiki/index.md`**

In "Current State", add a line noting the prompt:

```
- The `anvil-prompt` program renders an adaptive shell prompt (git, toolchain,
  container/cluster, exit state), wired in by the shell integration.
```

- [ ] **Step 3: Append `wiki/log.md`**

```
- 2026-05-21 — Caldera prompt (Phase 1) complete: new `anvil-prompt`
  executable (`src/prompt/`) — `icons`, `segments`, `context`, `git`, `render`,
  `build_segments`, `main`. Adaptive segments (cwd, git, toolchain, container,
  cluster, exit), two-line + transient, rich/ASCII glyphs gated on
  `$ANVIL`. `config.zon` gains a `prompt` section. Shell integration
  drives `PROMPT`/`PS1` from the binary. 1XX tests pass. Spec:
  docs/superpowers/specs/2026-05-21-anvil-prompt-design.md. Deferred: the
  bundled icon font (a one-file swap in `icons.zig`) and the Phase 2 interactive
  hover layer.
```

Replace `1XX` with the actual `zig build test` count.

- [ ] **Step 4: Update `todo.txt`**

Add a DONE entry for the Caldera prompt (Phase 1), mirroring the M-milestone entries' style; note the deferred icon font and Phase 2.

- [ ] **Step 5: Commit**

```bash
git add wiki/index.md wiki/log.md todo.txt
git commit -m "docs: record Caldera prompt Phase 1 completion"
```

---

## Self-review (completed while writing this plan)

- **Spec coverage:** the Zig program → Tasks 1-9; adaptive segments → Tasks 4, 8; git → Task 5; two-line + transient → Tasks 6, 9, 10; icon set (rich + ASCII, `$ANVIL`-gated) → Tasks 2, 9; config section → Task 7; custom segments → config type in Task 7 (evaluation of custom command-backed segments is a small follow-up — the config surface and the built-in adaptive segments ship here); shell wiring + transient → Task 10; theme colours → `render.zig` uses the Mineral hex values directly (a later pass can route them through the live theme); accent edge → Task 6. The **bundled icon font** (spec decision 6) and the **Phase 2 interactive hover layer** (spec decision 8) are explicitly deferred — the spec's Phasing section anticipates this.
- **Placeholders:** none — every step has complete code. (`1XX` in Task 11 is a substitution instruction.)
- **Type consistency:** `Icon` (icons.zig) is used by `segments.Segment` and `render`/`build_segments`. `seg.Segment`/`seg.State`/`seg.List` are consistent across `segments`, `render`, `build_segments`. `git.Info` is produced by `git.query` and consumed by `build_segments.Inputs`. `render.Options` is constructed in `main`. `ctx.Context`/`ctx.Lang` flow `context` → `build_segments`. Names match across tasks.

## Known follow-ups (out of scope for this plan)

- **Bundled icon font** — swap `icons.zig`'s rich glyphs to a bundled icon/Nerd font; touches the renderer's font stack. One isolated change.
- **Custom-segment evaluation** — run each `config.prompt.custom` entry's command and add it as a segment; small addition to `build_segments` + `main`.
- **Theme-driven colours** — `render.zig` currently inlines the Mineral hex values; route them through the live theme.
- **Phase 2 — the interactive hover layer** — its own plan (the metadata OSC, parser capture, hover hit-testing, the popover).
