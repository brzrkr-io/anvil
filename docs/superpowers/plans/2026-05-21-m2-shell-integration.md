# Shell Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship zsh + bash scripts that emit OSC 133 prompt marks and OSC 7 cwd, auto-loaded into spawned zsh shells.

**Architecture:** Two shell scripts plus a zsh `ZDOTDIR` shim are `@embedFile`d into the binary. `shell_integration.zig` writes them to `~/.cache/anvil/shell/` at startup and `setenv`s the wiring vars; `Pty.buildChildEnv` already copies `environ` into every child shell.

**Tech Stack:** Zig 0.16, zsh/bash, the M1 PTY layer, the M2 config system.

**Spec:** `docs/superpowers/specs/2026-05-21-shell-integration-design.md` — read it first.

**Branch:** Continue on `feat/m2-config-theme` (final sub-project; no new branch).

**Repo facts:** `Pty.buildChildEnv` (`src/pty/pty.zig`) builds the child env from `std.c.environ`, so env vars `setenv`'d before a tab spawns are inherited. `config.zig` uses `std.c` POSIX primitives (`open`/`read`/`close`, `getenv`) for filesystem/env work because Zig 0.16's high-level `std.Io` API needs an `Io` parameter — `shell_integration.zig` should follow that same `std.c` pattern (`std.c.mkdir`, `std.c.open`/`write`/`close`, `std.c.getenv`, `std.c.setenv`).

---

## Task 1: The shell scripts

**Files:**
- Create: `shell/anvil-integration.zsh`
- Create: `shell/anvil-integration.bash`
- Create: `shell/zdotdir-zshenv.zsh`

Static script files — no Zig, no tests in this task. They are `@embedFile`d in Task 3.

- [ ] **Step 1: Write `shell/anvil-integration.zsh`**

```zsh
# Anvil — zsh shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Sourced by the Caldera ZDOTDIR shim; safe to source manually too.

[[ -n "$ANVIL_ZSH_LOADED" ]] && return
ANVIL_ZSH_LOADED=1

# precmd: the previous command finished (133;D + exit), a new prompt starts
# (133;A), and report the cwd (OSC 7).
__anvil_precmd() {
  local last=$?
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# preexec: a command is about to run (133;C).
__anvil_preexec() {
  printf '\e]133;C\a'
}

typeset -ag precmd_functions preexec_functions
precmd_functions+=(__anvil_precmd)
preexec_functions+=(__anvil_preexec)

# 133;B marks the end of the prompt / start of typed input. Append it to PS1
# as a zero-width segment. Done from a one-shot precmd so it runs *after* the
# user's .zshrc has set PS1, then removes itself.
__anvil_mark_prompt() {
  if [[ "$PS1" != *'133;B'* ]]; then
    PS1="${PS1}%{$'\e]133;B\a'%}"
  fi
  precmd_functions=(${precmd_functions:#__anvil_mark_prompt})
}
precmd_functions+=(__anvil_mark_prompt)
```

- [ ] **Step 2: Write `shell/anvil-integration.bash`**

```bash
# Anvil — bash shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Opt-in: add to ~/.bashrc:
#   [ -n "$ANVIL" ] && [ -r "$ANVIL_SHELL_INTEGRATION" ] && . "$ANVIL_SHELL_INTEGRATION"

[ -n "$ANVIL_BASH_LOADED" ] && return
ANVIL_BASH_LOADED=1

__anvil_precmd() {
  local last=$?
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# DEBUG fires before every simple command; suppress it while PROMPT_COMMAND
# itself runs so only real commands emit 133;C.
__anvil_preexec() {
  [ -n "$__anvil_in_prompt" ] && return
  printf '\e]133;C\a'
}

__anvil_prompt_wrapper() {
  __anvil_in_prompt=1
  __anvil_precmd
  unset __anvil_in_prompt
}

PROMPT_COMMAND="__anvil_prompt_wrapper${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
trap '__anvil_preexec' DEBUG

case "$PS1" in
  *'133;B'*) ;;
  *) PS1="${PS1}\[\e]133;B\a\]" ;;
esac
```

- [ ] **Step 3: Write `shell/zdotdir-zshenv.zsh`**

This is the `.zshenv` written into the Caldera `ZDOTDIR`. zsh reads it first.

```zsh
# Anvil — zsh ZDOTDIR shim.
# zsh reads $ZDOTDIR/.zshenv first. Restore the real ZDOTDIR so the rest of
# zsh startup (.zprofile/.zshrc/.zlogin) reads the user's own files, run the
# user's real .zshenv, then load the Caldera integration.

ZDOTDIR="${ANVIL_REAL_ZDOTDIR:-$HOME}"
unset ANVIL_REAL_ZDOTDIR

[ -f "$ZDOTDIR/.zshenv" ] && source "$ZDOTDIR/.zshenv"

[ -n "$ANVIL_SHELL_INTEGRATION_ZSH" ] && [ -r "$ANVIL_SHELL_INTEGRATION_ZSH" ] && \
  source "$ANVIL_SHELL_INTEGRATION_ZSH"
```

- [ ] **Step 4: Verify the scripts are syntactically valid**

Run: `zsh -n shell/anvil-integration.zsh && zsh -n shell/zdotdir-zshenv.zsh && bash -n shell/anvil-integration.bash`
Expected: no output, exit 0 (all three parse clean).

- [ ] **Step 5: Commit**

```bash
git add shell/
git commit -m "feat(shell): zsh and bash integration scripts"
```

(End every commit message in this plan with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.)

---

## Task 2: `shell_integration` config field

**Files:**
- Modify: `src/config/config.zig`

- [ ] **Step 1: Add the field to `Config`**

Add to the `Config` struct (alongside `scrollback`, `theme`, etc.):

```zig
    shell_integration: bool = true,
```

- [ ] **Step 2: Write the failing test** (in `config.zig`'s test block)

```zig
test "config parses shell_integration" {
    var on = try parseSlice(testing.allocator, ".{ .scrollback = 100 }");
    defer on.deinit();
    try testing.expectEqual(true, on.config.shell_integration); // default

    var off = try parseSlice(testing.allocator, ".{ .shell_integration = false }");
    defer off.deinit();
    try testing.expectEqual(false, off.config.shell_integration);
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures.

- [ ] **Step 4: Commit**

```bash
git add src/config/config.zig
git commit -m "feat(config): shell_integration toggle"
```

---

## Task 3: `shell_integration.zig` — embed, write, env setup

**Files:**
- Create: `src/app/shell_integration.zig`
- Modify: `src/main.zig` (add `_ = @import("app/shell_integration.zig");` to the `test {}` block)

The module that `@embedFile`s the three scripts, writes them to the runtime dir, and `setenv`s the wiring vars.

- [ ] **Step 1: Write `src/app/shell_integration.zig`**

Follow `config.zig`'s `std.c` pattern for filesystem/env calls. Confirm `std.c.mkdir`, `std.c.setenv`, `std.c.getenv`, `std.c.open`/`write`/`close` signatures against the installed std (`/opt/homebrew/Cellar/zig/0.16.0_1/lib/zig/std/c.zig`).

```zig
//! Shell integration setup: write the embedded zsh/bash scripts to a runtime
//! dir and export the env vars that wire spawned shells to them.
//!
//! `Pty.buildChildEnv` copies `environ` into every child, so env vars exported
//! here (before any tab spawns) are inherited by every shell.

const std = @import("std");

const integration_zsh = @embedFile("../../shell/anvil-integration.zsh");
const integration_bash = @embedFile("../../shell/anvil-integration.bash");
const zdotdir_zshenv = @embedFile("../../shell/zdotdir-zshenv.zsh");

/// Resolve `~/.cache/anvil/shell` into `buf`. Null when `$HOME`
/// is unset.
fn runtimeDir(buf: []u8) ?[]const u8 {
    const home = std.c.getenv("HOME") orelse return null;
    const h = std.mem.span(home);
    return std.fmt.bufPrint(buf, "{s}/.cache/anvil/shell", .{h}) catch null;
}

/// Create every directory along `path` (like `mkdir -p`). Best-effort.
fn mkdirP(path: []const u8) void {
    var i: usize = 1;
    while (i < path.len) : (i += 1) {
        if (path[i] != '/') continue;
        var seg: [std.fs.max_path_bytes]u8 = undefined;
        if (i >= seg.len) return;
        @memcpy(seg[0..i], path[0..i]);
        seg[i] = 0;
        _ = std.c.mkdir(seg[0..i :0].ptr, 0o755);
    }
    var full: [std.fs.max_path_bytes]u8 = undefined;
    if (path.len >= full.len) return;
    @memcpy(full[0..path.len], path);
    full[path.len] = 0;
    _ = std.c.mkdir(full[0..path.len :0].ptr, 0o755);
}

/// Write `content` to `dir/name`. Returns false on any failure.
fn writeFile(dir: []const u8, name: []const u8, content: []const u8) bool {
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    const path = std.fmt.bufPrintZ(&pbuf, "{s}/{s}", .{ dir, name }) catch return false;
    const fd = std.c.open(path.ptr, .{ .ACCMODE = .WRONLY, .CREAT = true, .TRUNC = true }, 0o644);
    if (fd < 0) return false;
    defer _ = std.c.close(fd);
    var off: usize = 0;
    while (off < content.len) {
        const n = std.c.write(fd, content[off..].ptr, content.len - off);
        if (n <= 0) return false;
        off += @intCast(n);
    }
    return true;
}

/// Set up shell integration. Writes the scripts and exports the wiring env
/// vars. When `enabled` is false, exports only the harmless markers and skips
/// `ZDOTDIR`. Any filesystem failure is logged and degrades to "no
/// integration" — never fatal. Call once at startup, before any tab spawns.
pub fn setup(enabled: bool) void {
    var dbuf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&dbuf) orelse {
        std.debug.print("anvil: shell integration: $HOME unset, skipped\n", .{});
        return;
    };
    mkdirP(dir);

    const ok_zsh = writeFile(dir, "anvil-integration.zsh", integration_zsh);
    const ok_bash = writeFile(dir, "anvil-integration.bash", integration_bash);
    const ok_env = writeFile(dir, ".zshenv", zdotdir_zshenv);
    if (!(ok_zsh and ok_bash and ok_env)) {
        std.debug.print("anvil: shell integration: write failed, skipped\n", .{});
        return;
    }

    // Markers — always exported; harmless to any shell.
    _ = std.c.setenv("ANVIL", "1", 1);
    var bbuf: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&bbuf, "{s}/anvil-integration.bash", .{dir})) |bash_path| {
        _ = std.c.setenv("ANVIL_SHELL_INTEGRATION", bash_path.ptr, 1);
    } else |_| {}

    if (!enabled) return;

    // zsh auto-injection: point ZDOTDIR at our dir, after stashing the real one.
    const real = std.c.getenv("ZDOTDIR");
    if (real) |r| {
        _ = std.c.setenv("ANVIL_REAL_ZDOTDIR", r, 1);
    }
    var zbuf: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&zbuf, "{s}/anvil-integration.zsh", .{dir})) |zsh_path| {
        _ = std.c.setenv("ANVIL_SHELL_INTEGRATION_ZSH", zsh_path.ptr, 1);
    } else |_| {}
    var dz: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&dz, "{s}", .{dir})) |dirz| {
        _ = std.c.setenv("ZDOTDIR", dirz.ptr, 1);
    } else |_| {}
}
```

Note: verify the `std.c.open` flags struct form against the installed std — `config.zig`'s `load`/`Watcher` already call `std.c.open`; mirror exactly how `config.zig` opens a file (it opens read-only; here you need write/create/truncate — use the same flag-struct style the std exposes). If the flag form differs, match the real `std.c.O` definition.

- [ ] **Step 2: Add the module to the test aggregator**

In `src/main.zig`'s `test { }` block add:

```zig
    _ = @import("app/shell_integration.zig");
```

- [ ] **Step 3: Write the failing tests** (end of `src/app/shell_integration.zig`)

The tests drive `setup` with a temp `$HOME` so the real home is untouched.

```zig
const testing = std.testing;

test "runtimeDir resolves under HOME" {
    // Save and override HOME.
    const saved = std.c.getenv("HOME");
    _ = std.c.setenv("HOME", "/tmp/caldera-shell-test", 1);
    defer if (saved) |s| {
        _ = std.c.setenv("HOME", s, 1);
    };
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&buf).?;
    try testing.expectEqualStrings("/tmp/caldera-shell-test/.cache/anvil/shell", dir);
}

test "setup writes the scripts and exports markers" {
    const saved_home = std.c.getenv("HOME");
    _ = std.c.setenv("HOME", "/tmp/caldera-shell-test", 1);
    defer if (saved_home) |s| {
        _ = std.c.setenv("HOME", s, 1);
    };

    setup(true);

    // The three files exist and are non-empty.
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&buf).?;
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    inline for (.{ "anvil-integration.zsh", "anvil-integration.bash", ".zshenv" }) |name| {
        const path = try std.fmt.bufPrintZ(&pbuf, "{s}/{s}", .{ dir, name });
        const fd = std.c.open(path.ptr, .{ .ACCMODE = .RDONLY }, 0);
        try testing.expect(fd >= 0);
        _ = std.c.close(fd);
    }

    // Markers exported.
    try testing.expect(std.c.getenv("ANVIL") != null);
    try testing.expect(std.c.getenv("ZDOTDIR") != null);
}

test "setup(false) does not export ZDOTDIR" {
    const saved_home = std.c.getenv("HOME");
    _ = std.c.setenv("HOME", "/tmp/caldera-shell-test-2", 1);
    defer if (saved_home) |s| {
        _ = std.c.setenv("HOME", s, 1);
    };
    // Clear any ZDOTDIR a prior test set.
    _ = std.c.unsetenv("ZDOTDIR");

    setup(false);
    try testing.expect(std.c.getenv("ANVIL") != null); // marker still set
    try testing.expect(std.c.getenv("ZDOTDIR") == null);         // but no injection
}
```

Note: these tests mutate process env and write to `/tmp`. That is acceptable for an integration-style unit test. If `std.c.unsetenv` is unavailable, set `ZDOTDIR` to a known sentinel and assert it is unchanged instead. Run the `setup(false)` test logic so it does not depend on test ordering — give it its own temp HOME (done above).

- [ ] **Step 4: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures. If `@embedFile` paths are wrong, fix them relative to `src/app/shell_integration.zig` (the `shell/` dir is at the repo root, so `../../shell/...`).

- [ ] **Step 5: Commit**

```bash
git add src/app/shell_integration.zig src/main.zig
git commit -m "feat(shell): embed scripts, write runtime dir, export env"
```

---

## Task 4: Wire `setup` into `main.zig`

**Files:**
- Modify: `src/main.zig`

- [ ] **Step 1: Call `setup` at startup**

Add the import near the other `app/` imports:

```zig
const shell_integration = @import("app/shell_integration.zig");
```

In `main`, after the config is loaded (`cfg` is available) and **before** the `TabManager` / first tab is created, add:

```zig
    shell_integration.setup(cfg.shell_integration);
```

The first tab's shell — and every later tab's — then inherits the exported env via `Pty.buildChildEnv`.

- [ ] **Step 2: Build, test, run**

Run: `zig build test --summary all` — all tests pass, zero failures.
Run: `zig build` — exit 0, no warnings.
Run: `( zig build run & sleep 5; kill %1 2>/dev/null )` then `pkill -f 'zig-out/bin/anvil'` — confirm the app launches without crashing. If you cannot run the GUI, confirm `zig build` is clean and say so.

- [ ] **Step 3: Commit**

```bash
git add src/main.zig
git commit -m "feat(shell): enable shell integration at startup"
```

---

## Task 5: End-to-end verification and closeout

**Files:**
- Modify: `todo.txt`, `wiki/`

- [ ] **Step 1: Full test run**

Run: `zig build test --summary all` — every test passes, zero failures.

- [ ] **Step 2: Interactive verification**

Run `zig build run`. In the spawned zsh:
- Type a command (e.g. `ls`) and run it; then `cd` somewhere.
- The integration is invisible on screen — verify it works indirectly:
  - Open a new tab (⌘T): it should open in the directory you `cd`'d to (OSC 7
    set the cwd, which `addTab` reads). This is the visible proof.
  - The new tab's label reflects the cwd basename (when no shell title is set).
- Confirm normal shell behavior is intact: your aliases, prompt, and `$PATH`
  all still work (the ZDOTDIR shim sourced your real config).

Capture a screenshot showing two tabs whose labels are directory names. Do not
leave anything in the real `~/.config/anvil/`. The runtime dir
`~/.cache/anvil/shell/` is expected and fine to leave.

- [ ] **Step 3: Close out docs**

- `todo.txt`: check off the M2 shell-integration item — **this completes M2**.
  Update the milestone line (M2 done).
- `wiki/`: add `wiki/concepts/shell-integration.md` (frontmatter per
  `wiki/index.md`) — the OSC 133/7 marks, the `ZDOTDIR` shim mechanism, the
  bash opt-in line, the `shell_integration` config toggle. Link it from
  `wiki/index.md`. Update `wiki/index.md`'s "Current State" to M2 complete.
- Append a `wiki/log.md` entry for the completed sub-project and for M2.

- [ ] **Step 4: Commit**

```bash
git add todo.txt wiki/
git commit -m "docs: record M2 shell-integration sub-project; M2 complete"
```

---

## Done criteria

- `zig build test` passes; all sub-project 1-4 tests are green.
- `zig build run`: a spawned zsh emits OSC 133 marks and OSC 7 cwd — verified
  via new-tab-inherits-cwd and `Terminal.promptMarks()` populating; the user's
  own zsh config still loads.
- This is the **last M2 sub-project** — M2 (multi-tab, search, shell
  integration, config/theme) is complete.
