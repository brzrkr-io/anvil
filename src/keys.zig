// Keyboard shortcut table. Mirrors the dispatch in src/platform/shim.m keyDown
// and MUST be kept in sync with it by hand.

const std = @import("std");

pub const Binding = struct { chord: []const u8, action: []const u8 };
pub const Section = struct { title: []const u8, items: []const Binding };

const general = [_]Binding{
    .{ .chord = "\u{2318}N", .action = "New Window" },
    .{ .chord = "\u{2318}K", .action = "Command Palette" },
    .{ .chord = "\u{2318}F", .action = "Search Scrollback" },
    .{ .chord = "\u{2318}/", .action = "Keyboard Shortcuts" },
    .{ .chord = "\u{2318}C", .action = "Copy" },
    .{ .chord = "\u{2318}V", .action = "Paste" },
};

const search_bindings = [_]Binding{
    .{ .chord = "\u{21a9}/\u{21e7}\u{21a9}", .action = "Next / Prev Match" },
    .{ .chord = "Tab", .action = "Toggle Regex Mode" },
    .{ .chord = "Esc", .action = "Close Search" },
};

const panes = [_]Binding{
    .{ .chord = "\u{2318}D", .action = "Split Right" },
    .{ .chord = "\u{2318}\u{21e7}D", .action = "Split Down" },
    .{ .chord = "\u{2318}W", .action = "Close Pane" },
    .{ .chord = "\u{2318}\u{21e7}\u{23ce}", .action = "Zoom Pane" },
    .{ .chord = "\u{2318}\u{2325}=", .action = "Balance Panes" },
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
    .{ .chord = "\u{2318}1\u{2013}9", .action = "Jump to Tab" },
};

const terminal = [_]Binding{
    .{ .chord = "\u{2318}\u{2191}", .action = "Jump to Prev Prompt" },
    .{ .chord = "\u{2318}\u{2193}", .action = "Jump to Next Prompt" },
    .{ .chord = "\u{2318}R", .action = "Restart Shell" },
    .{ .chord = "\u{2318}\u{21e7}Space", .action = "Copy Mode" },
};

const copy_mode_bindings = [_]Binding{
    .{ .chord = "h/j/k/l", .action = "Move Caret" },
    .{ .chord = "Arrows", .action = "Move Caret" },
    .{ .chord = "w / b", .action = "Word Forward / Back" },
    .{ .chord = "g / G", .action = "Top / Bottom" },
    .{ .chord = "^U / ^D", .action = "Half Page Up / Down" },
    .{ .chord = "v", .action = "Start Visual Selection" },
    .{ .chord = "y / Enter", .action = "Copy Selection & Exit" },
    .{ .chord = "Esc / q", .action = "Exit Copy Mode" },
};

const agents = [_]Binding{
    .{ .chord = "\u{2318}G", .action = "Open Run Drawer" },
    .{ .chord = "\u{2191}/\u{2193}", .action = "Select Run" },
    .{ .chord = "Esc", .action = "Close Drawer" },
};

const view = [_]Binding{
    .{ .chord = "\u{2318}=", .action = "Zoom In" },
    .{ .chord = "\u{2318}-", .action = "Zoom Out" },
    .{ .chord = "\u{2318}0", .action = "Reset Zoom" },
};

pub const sections = [_]Section{
    .{ .title = "General", .items = &general },
    .{ .title = "Search", .items = &search_bindings },
    .{ .title = "Panes", .items = &panes },
    .{ .title = "Tabs", .items = &tabs },
    .{ .title = "Terminal", .items = &terminal },
    .{ .title = "Copy Mode", .items = &copy_mode_bindings },
    .{ .title = "Agents", .items = &agents },
    .{ .title = "View", .items = &view },
};

pub const total_bindings: usize = general.len + search_bindings.len + panes.len + tabs.len + terminal.len + copy_mode_bindings.len + agents.len + view.len;

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
    try std.testing.expectEqual(@as(usize, 45), total_bindings);
}
