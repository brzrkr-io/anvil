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
