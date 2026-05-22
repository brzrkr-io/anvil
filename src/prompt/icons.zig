//! Prompt icon glyphs. `rich` glyphs are Nerd Font v3 codepoints, carried by
//! the bundled BlexMono Nerd Font Mono; `ascii` fallbacks render anywhere. The
//! two-form table is the single swap point for icon rendering.

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
        .repo => if (rich) "\u{f07b}" else "#", // nf-fa-folder
        .branch => if (rich) "\u{e0a0}" else "@", // nf-pl-branch
        .dirty => if (rich) "\u{f111}" else "*", // nf-fa-circle
        .ahead => if (rich) "\u{f062}" else "^", // nf-fa-arrow_up
        .behind => if (rich) "\u{f063}" else "v", // nf-fa-arrow_down
        .toolchain => if (rich) "\u{f085}" else "=", // nf-fa-cogs
        .container => if (rich) "\u{f308}" else "[]", // nf-linux-docker
        .cluster => if (rich) "\u{f10fe}" else "{}", // nf-md-kubernetes
        .ok => if (rich) "\u{f00c}" else "ok", // nf-fa-check
        .err => if (rich) "\u{f00d}" else "x", // nf-fa-close
        .clock => if (rich) "\u{f017}" else "@", // nf-fa-clock_o
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
