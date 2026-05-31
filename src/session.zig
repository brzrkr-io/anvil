const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const Parser = @import("vt/parser.zig").Parser;
const Pty = @import("pty.zig").Pty;
const syntax = @import("syntax.zig");
const fileview = @import("fileview.zig");
const Editor = @import("editor.zig").Editor;
const WebPane = @import("web/pane.zig").WebPane;
const Color = @import("vt/cell.zig").Color;

pub const Kind = enum { shell, viewer, editor, web };

fn roleColor(role: syntax.Role) Color {
    return switch (role) {
        .keyword => .{ .indexed = 13 },
        .string => .{ .indexed = 2 },
        .number => .{ .indexed = 3 },
        .comment => .{ .indexed = 8 },
        .type => .{ .indexed = 6 },
        .punct => .default,
        .text => .default,
    };
}

/// One terminal session: a VT emulator, its parser, and (for shell kind) a PTY.
pub const Session = struct {
    id: usize = 0,
    term: Terminal,
    parser: Parser = .{},
    pty: Pty,
    exited: bool = false,
    kind: Kind = .shell,
    // Viewer state: retained file bytes owned by the session's allocator.
    view_bytes: []u8 = &[_]u8{},
    view_lang: syntax.Lang = .unknown,
    view_alloc: std.mem.Allocator = undefined,
    // Editor state: a native editable buffer (editor kind only).
    editor: ?Editor = null,
    // Web state: pure display state; app.zig owns the WKWebView handle.
    web: ?WebPane = null,

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Session {
        return initWithCwd(alloc, rows, cols, null);
    }

    pub fn initWithCwd(alloc: std.mem.Allocator, rows: u16, cols: u16, cwd: ?[*:0]const u8) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        var pty = try Pty.spawnCwd(rows, cols, cwd);
        pty.setNonblock();
        return .{ .term = term, .pty = pty };
    }

    /// Create a viewer session (no PTY). The caller must call fillGrid after
    /// setting view_bytes/view_lang, or use initViewer.
    pub fn initViewer(alloc: std.mem.Allocator, rows: u16, cols: u16) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        const pty = Pty.initNull();
        return .{
            .term = term,
            .pty = pty,
            .kind = .viewer,
            .view_alloc = alloc,
        };
    }

    /// Create an editor session (no PTY) holding a native editable buffer
    /// loaded from `path`. The buffer is rendered into the grid immediately.
    pub fn initEditor(alloc: std.mem.Allocator, rows: u16, cols: u16, path: []const u8) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        const pty = Pty.initNull();
        var ed = try Editor.initEmpty(alloc);
        errdefer ed.deinit();
        try ed.loadFile(path);
        var s = Session{
            .term = term,
            .pty = pty,
            .kind = .editor,
            .editor = ed,
            .view_alloc = alloc,
        };
        s.editor.?.render(&s.term, rows, cols);
        return s;
    }

    /// Create a web (WKWebView) pane session. Holds only pure display state;
    /// app.zig creates and owns the native WKWebView once this pane lays out.
    pub fn initWeb(alloc: std.mem.Allocator, rows: u16, cols: u16, url: []const u8) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        const pty = Pty.initNull();
        return .{
            .term = term,
            .pty = pty,
            .kind = .web,
            .web = WebPane.init(url),
            .view_alloc = alloc,
        };
    }

    pub fn deinit(self: *Session) void {
        switch (self.kind) {
            .viewer => if (self.view_bytes.len > 0) self.view_alloc.free(self.view_bytes),
            .editor => if (self.editor) |*e| e.deinit(),
            .shell => self.pty.deinit(),
            .web => {},
        }
        self.term.deinit();
    }

    pub fn resize(self: *Session, rows: u16, cols: u16) !void {
        try self.term.resize(rows, cols);
        switch (self.kind) {
            .shell => self.pty.resize(rows, cols),
            .viewer => self.fillGrid(),
            .editor => if (self.editor) |*e| e.render(&self.term, rows, cols),
            .web => {},
        }
    }

    pub const PollResult = struct { alive: bool, consumed: bool };

    /// Drain pending shell output into the terminal.
    /// Viewer and editor sessions return immediately (alive, nothing consumed).
    pub fn poll(self: *Session) PollResult {
        if (self.kind != .shell) return .{ .alive = true, .consumed = false };
        var buf: [8192]u8 = undefined;
        var consumed = false;
        while (true) {
            switch (self.pty.read(&buf)) {
                .data => |n| {
                    consumed = true;
                    self.parser.feed(&self.term, buf[0..n]);
                    if (self.term.reply_len > 0) {
                        self.pty.write(self.term.reply_buf[0..self.term.reply_len]);
                        self.term.reply_len = 0;
                    }
                },
                .would_block => return .{ .alive = true, .consumed = consumed },
                .eof => return .{ .alive = false, .consumed = consumed },
            }
        }
    }

    /// Re-fork the PTY and reset the terminal. Called after the shell exits.
    pub fn respawn(self: *Session) !void {
        self.pty.deinit();
        self.pty = try Pty.spawn(self.term.grid.rows, self.term.grid.cols);
        self.pty.setNonblock();
        self.term.reset();
        self.parser = .{};
        self.exited = false;
    }

    /// Send input to the shell; typing jumps to the live view and clears selection.
    /// No-op for viewer and editor sessions (editor input goes through editorInput).
    pub fn write(self: *Session, bytes: []const u8) void {
        if (self.kind != .shell) return;
        if (self.term.view_offset != 0) self.term.view_offset = 0;
        self.term.clearSelection();
        self.pty.write(bytes);
    }

    /// Feed key bytes to the editor and repaint the grid. No-op otherwise.
    pub fn editorInput(self: *Session, bytes: []const u8) !void {
        if (self.editor) |*e| {
            try e.input(bytes);
            e.render(&self.term, self.term.grid.rows, self.term.grid.cols);
        }
    }

    /// Save the editor buffer to its file. Errors if there is no editor/path.
    pub fn editorSave(self: *Session) !void {
        if (self.editor) |*e| try e.save() else return error.NotAnEditor;
    }

    /// Replace this editor pane's buffer with `path`, reusing the pane. Lets the
    /// explorer open files into one editor instead of splitting a new pane each
    /// click. Errors (binary/too large/missing) leave the old buffer intact.
    pub fn reloadEditor(self: *Session, path: []const u8) !void {
        if (self.kind != .editor) return error.NotAnEditor;
        var ed = try Editor.initEmpty(self.view_alloc);
        errdefer ed.deinit();
        try ed.loadFile(path);
        if (self.editor) |*old| old.deinit();
        self.editor = ed;
        self.editor.?.render(&self.term, self.term.grid.rows, self.term.grid.cols);
    }

    /// Place the editor cursor from a grid click and repaint.
    pub fn editorClick(self: *Session, row: usize, col: usize) void {
        if (self.editor) |*e| {
            e.click(row, col, self.term.grid.rows);
            e.render(&self.term, self.term.grid.rows, self.term.grid.cols);
        }
    }

    /// Scroll the editor viewport by `delta` lines and repaint.
    pub fn editorScroll(self: *Session, delta: i32) void {
        if (self.editor) |*e| {
            e.scroll(delta, self.term.grid.rows);
            e.render(&self.term, self.term.grid.rows, self.term.grid.cols);
        }
    }

    /// Tokenize view_bytes and write colored cells into the terminal grid.
    /// Called after load and after resize. Resets the terminal first.
    pub fn fillGrid(self: *Session) void {
        self.term.reset();
        if (self.view_bytes.len == 0) return;

        const cols = self.term.grid.cols;
        var line_bufs: [8192][]const u8 = undefined;
        const n_lines = fileview.splitLines(self.view_bytes, &line_bufs);
        var tok_buf: [512]syntax.Token = undefined;

        var li: usize = 0;
        while (li < n_lines) : (li += 1) {
            const line = line_bufs[li];
            const n_toks = syntax.tokenizeLine(self.view_lang, line, &tok_buf);

            // Emit tokens, truncating at grid width.
            var col: u16 = 0;
            var ti: usize = 0;
            while (ti < n_toks and col < cols) : (ti += 1) {
                const tok = tok_buf[ti];
                const tok_bytes = line[tok.start .. tok.start + tok.len];
                self.term.pen.fg = roleColor(tok.role);
                for (tok_bytes) |byte| {
                    if (col >= cols) break;
                    self.term.print(byte);
                    col += 1;
                }
            }

            // Reset pen and emit newline (lineFeed + carriageReturn).
            self.term.pen.fg = .default;
            self.term.carriageReturn();
            self.term.lineFeed();
        }

        // Scroll to top so the first line is visible.
        const sb = self.term.scrollback.len();
        if (sb > 0) self.term.view_offset = sb;
    }
};

test "editor session: load renders grid, type + save round-trips through disk" {
    const alloc = std.testing.allocator;
    const path = "/tmp/anvil_session_editor_test.zig";
    try fileview.save(path, "const x = 1;\n");
    defer _ = std.c.unlink(path);

    var s = try Session.initEditor(alloc, 10, 40, path);
    defer s.deinit();
    try std.testing.expectEqual(Kind.editor, s.kind);
    // The buffer is rendered into the grid on open.
    try std.testing.expectEqual(@as(u21, 'c'), s.term.grid.at(0, 3).cp); // content past gw=3
    try std.testing.expectEqual(@as(u16, 0), s.term.cy);
    try std.testing.expectEqual(@as(u16, 3), s.term.cx); // cur_col 0 + gw 3

    try s.editorInput("\x1b[F"); // jump to end of line
    try s.editorInput(" // note");
    try s.editorSave();

    var s2 = try Session.initEditor(alloc, 10, 40, path);
    defer s2.deinit();
    try std.testing.expectEqualStrings("const x = 1; // note", s2.editor.?.lines.items[0].items);
}

test "reloadEditor swaps the buffer in place, repainting the grid" {
    const alloc = std.testing.allocator;
    const a = "/tmp/anvil_reload_a.zig";
    const b = "/tmp/anvil_reload_b.zig";
    try fileview.save(a, "alpha\n");
    try fileview.save(b, "bravo\n");
    defer _ = std.c.unlink(a);
    defer _ = std.c.unlink(b);

    var s = try Session.initEditor(alloc, 10, 40, a);
    defer s.deinit();
    try std.testing.expectEqualStrings("alpha", s.editor.?.lines.items[0].items);
    try std.testing.expectEqual(@as(u21, 'a'), s.term.grid.at(0, 3).cp);

    try s.reloadEditor(b);
    try std.testing.expectEqualStrings("bravo", s.editor.?.lines.items[0].items);
    // Grid repainted with the new file's first glyph.
    try std.testing.expectEqual(@as(u21, 'b'), s.term.grid.at(0, 3).cp);
}

test "web session: initWeb sets kind + url, poll is inert" {
    const alloc = std.testing.allocator;
    var s = try Session.initWeb(alloc, 10, 40, "https://example.com");
    defer s.deinit();
    try std.testing.expectEqual(Kind.web, s.kind);
    try std.testing.expectEqualStrings("https://example.com", s.web.?.url());
    const r = s.poll();
    try std.testing.expect(r.alive and !r.consumed);
}

test "poll: consumed=true on data, alive=false on eof" {
    var s = try Session.init(std.testing.allocator, 24, 80);
    defer s.term.deinit();
    s.pty.write("exit\n");
    var got_consumed = false;
    var got_eof = false;
    var iters: usize = 0;
    while (iters < 200) : (iters += 1) {
        const r = s.poll();
        if (r.consumed) got_consumed = true;
        if (!r.alive) {
            got_eof = true;
            break;
        }
        const ts = std.c.timespec{ .sec = 0, .nsec = 10 * std.time.ns_per_ms };
        _ = std.c.nanosleep(&ts, null);
    }
    try std.testing.expect(got_consumed);
    try std.testing.expect(got_eof);
}
