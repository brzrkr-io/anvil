// Keyboard shortcut table. Mirrors the dispatch in src/platform/shim.m keyDown
// and MUST be kept in sync with it by hand.

const std = @import("std");

pub const Binding = struct { chord: []const u8, action: []const u8 };
pub const Section = struct { title: []const u8, items: []const Binding };

const general = [_]Binding{
    .{ .chord = "\u{2318}K", .action = "Command Palette" },
    .{ .chord = "\u{2318}F", .action = "Search Scrollback" },
    .{ .chord = "\u{2318}/", .action = "Keyboard Shortcuts" },
    .{ .chord = "\u{2318}C", .action = "Copy" },
    .{ .chord = "\u{2318}V", .action = "Paste" },
};

const panes = [_]Binding{
    .{ .chord = "\u{2318}D", .action = "Split Right" },
    .{ .chord = "\u{2318}\u{21e7}D", .action = "Split Down" },
    .{ .chord = "\u{2318}W", .action = "Close Pane" },
    .{ .chord = "\u{2318}\u{21e7}\u{23ce}", .action = "Zoom Pane" },
    .{ .chord = "\u{2318}=", .action = "Balance Panes" },
    .{ .chord = "\u{2318}\u{2325}\u{2190}", .action = "Focus Left" },
    .{ .chord = "\u{2318}\u{2325}\u{2192}", .action = "Focus Right" },
    .{ .chord = "\u{2318}\u{2325}\u{2191}", .action = "Focus Up" },
    .{ .chord = "\u{2318}\u{2325}\u{2193}", .action = "Focus Down" },
    .{ .chord = "\u{2318}\u{21e7}\u{2190}", .action = "Resize Left" },
    .{ .chord = "\u{2318}\u{21e7}\u{2192}", .action = "Resize Right" },
    .{ .chord = "\u{2318}\u{21e7}\u{2191}", .action = "Resize Up" },
    .{ .chord = "\u{2318}\u{21e7}\u{2193}", .action = "Resize Down" },
};

const tabs = [_]Binding{
    .{ .chord = "\u{2318}T", .action = "New Tab" },
    .{ .chord = "\u{2318}\u{21e7}W", .action = "Close Tab" },
    .{ .chord = "\u{2318}[", .action = "Previous Tab" },
    .{ .chord = "\u{2318}]", .action = "Next Tab" },
};

const terminal = [_]Binding{
    .{ .chord = "\u{2318}\u{2191}", .action = "Jump to Prev Prompt" },
    .{ .chord = "\u{2318}\u{2193}", .action = "Jump to Next Prompt" },
};

pub const sections = [_]Section{
    .{ .title = "General", .items = &general },
    .{ .title = "Panes", .items = &panes },
    .{ .title = "Tabs", .items = &tabs },
    .{ .title = "Terminal", .items = &terminal },
};

pub const total_bindings: usize = general.len + panes.len + tabs.len + terminal.len;

test "sections non-empty" {
    try std.testing.expect(sections.len > 0);
}

test "all bindings have non-empty chord and action" {
    for (sections) |sec| {
        for (sec.items) |b| {
            try std.testing.expect(b.chord.len > 0);
            try std.testing.expect(b.action.len > 0);
        }
    }
}

test "total binding count" {
    try std.testing.expectEqual(@as(usize, 24), total_bindings);
}
