const std = @import("std");

/// Actions the command palette can launch. Each maps to a C-ABI export the
/// app already has; the app does the dispatch, the palette only picks one.
pub const ActionId = enum {
    split_side,
    split_stacked,
    close_pane,
    new_tab,
    next_tab,
    prev_tab,
    focus_left,
    focus_right,
    focus_up,
    focus_down,
    theme_system,
    theme_light,
    theme_dark,
    mode_terminal,
    mode_editor,
    mode_ide,
    open_web,
};

pub const Action = struct { id: ActionId, label: []const u8 };

pub const registry = [_]Action{
    .{ .id = .split_side, .label = "Split Pane Right" },
    .{ .id = .split_stacked, .label = "Split Pane Down" },
    .{ .id = .close_pane, .label = "Close Pane" },
    .{ .id = .new_tab, .label = "New Tab" },
    .{ .id = .next_tab, .label = "Next Tab" },
    .{ .id = .prev_tab, .label = "Previous Tab" },
    .{ .id = .focus_left, .label = "Focus Pane Left" },
    .{ .id = .focus_right, .label = "Focus Pane Right" },
    .{ .id = .focus_up, .label = "Focus Pane Up" },
    .{ .id = .focus_down, .label = "Focus Pane Down" },
    .{ .id = .theme_system, .label = "Theme: System" },
    .{ .id = .theme_light, .label = "Theme: Light" },
    .{ .id = .theme_dark, .label = "Theme: Dark" },
    .{ .id = .mode_terminal, .label = "Mode: Terminal" },
    .{ .id = .mode_editor, .label = "Mode: Editor" },
    .{ .id = .mode_ide, .label = "Mode: IDE" },
    .{ .id = .open_web, .label = "Open Browser Pane" },
};

/// Case-insensitive subsequence match. Returns a score (lower is better:
/// earlier and tighter matches win) or null when `query` is not a subsequence
/// of `label`.
pub fn score(label: []const u8, query: []const u8) ?u32 {
    if (query.len == 0) return 0;
    var qi: usize = 0;
    var s: u32 = 0;
    var last: ?usize = null;
    for (label, 0..) |ch, li| {
        if (qi >= query.len) break;
        if (lower(ch) == lower(query[qi])) {
            // Penalize the position of the first match and gaps between hits.
            if (last) |l| s += @intCast(li - l - 1) else s += @intCast(li);
            last = li;
            qi += 1;
        }
    }
    return if (qi == query.len) s else null;
}

fn lower(c: u8) u8 {
    return if (c >= 'A' and c <= 'Z') c + 32 else c;
}

const max_query = 64;

/// Command palette state: a query buffer, the filtered+ranked result list
/// (indices into `registry`), and the highlighted selection.
pub const Palette = struct {
    open: bool = false,
    query: [max_query]u8 = undefined,
    qlen: usize = 0,
    results: [registry.len]usize = undefined,
    count: usize = 0,
    sel: usize = 0,

    pub fn show(self: *Palette) void {
        self.open = true;
        self.qlen = 0;
        self.sel = 0;
        self.filter();
    }

    pub fn hide(self: *Palette) void {
        self.open = false;
    }

    pub fn typeChar(self: *Palette, c: u8) void {
        if (self.qlen >= max_query) return;
        self.query[self.qlen] = c;
        self.qlen += 1;
        self.filter();
    }

    pub fn backspace(self: *Palette) void {
        if (self.qlen == 0) return;
        self.qlen -= 1;
        self.filter();
    }

    pub fn moveDown(self: *Palette) void {
        if (self.count == 0) return;
        self.sel = (self.sel + 1) % self.count;
    }

    pub fn moveUp(self: *Palette) void {
        if (self.count == 0) return;
        self.sel = (self.sel + self.count - 1) % self.count;
    }

    pub fn selected(self: *const Palette) ?ActionId {
        if (self.count == 0) return null;
        return registry[self.results[self.sel]].id;
    }

    fn queryStr(self: *const Palette) []const u8 {
        return self.query[0..self.qlen];
    }

    /// Rebuild `results` from the registry, keeping only matches, sorted by
    /// score. Clamps the selection into range.
    fn filter(self: *Palette) void {
        const q = self.queryStr();
        var scores: [registry.len]u32 = undefined;
        self.count = 0;
        for (registry, 0..) |a, i| {
            if (score(a.label, q)) |sc| {
                self.results[self.count] = i;
                scores[self.count] = sc;
                self.count += 1;
            }
        }
        // Insertion sort the result indices by ascending score (stable, tiny n).
        var i: usize = 1;
        while (i < self.count) : (i += 1) {
            const ri = self.results[i];
            const sv = scores[i];
            var j = i;
            while (j > 0 and scores[j - 1] > sv) : (j -= 1) {
                self.results[j] = self.results[j - 1];
                scores[j] = scores[j - 1];
            }
            self.results[j] = ri;
            scores[j] = sv;
        }
        if (self.sel >= self.count) self.sel = if (self.count == 0) 0 else self.count - 1;
    }
};

test "score matches subsequence case-insensitively" {
    try std.testing.expect(score("New Tab", "nt") != null);
    try std.testing.expect(score("New Tab", "tab") != null);
    try std.testing.expect(score("New Tab", "xyz") == null);
}

test "score ranks earlier/tighter matches lower" {
    const early = score("Split Pane Right", "sp").?;
    const late = score("Focus Pane Left", "pl").?;
    try std.testing.expect(early < late);
}

test "empty query keeps the full registry in order" {
    var p = Palette{};
    p.show();
    try std.testing.expectEqual(registry.len, p.count);
    try std.testing.expectEqual(ActionId.split_side, p.selected().?);
}

test "typing filters and ranks results" {
    var p = Palette{};
    p.show();
    p.typeChar('t');
    p.typeChar('a');
    p.typeChar('b');
    try std.testing.expect(p.count > 0);
    // "New Tab" should be the top hit for "tab".
    try std.testing.expectEqual(ActionId.new_tab, p.selected().?);
}

test "backspace re-widens the result set" {
    var p = Palette{};
    p.show();
    p.typeChar('z'); // matches nothing
    try std.testing.expectEqual(@as(usize, 0), p.count);
    p.backspace();
    try std.testing.expectEqual(registry.len, p.count);
}

test "selection wraps and stays in range" {
    var p = Palette{};
    p.show();
    p.moveUp(); // wrap to last
    try std.testing.expectEqual(p.count - 1, p.sel);
    p.moveDown(); // wrap to first
    try std.testing.expectEqual(@as(usize, 0), p.sel);
}
