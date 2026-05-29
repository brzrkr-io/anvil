/// Per-cell GPU instance. Layout must match `Instance` in shaders.metal.
/// `x`/`y` are the cell's top-left in device pixels (pane origin baked in).
pub const CellInstance = extern struct {
    x: f32,
    y: f32,
    fg: [4]f32,
    bg: [4]f32,
    uv: [2]f32,
    /// Render flags drawn in-shader. bit0 underline, bit1 strike, bit2 dim.
    flags: u32 = 0,
};

pub const flag_underline: u32 = 1;
pub const flag_strike: u32 = 2;
pub const flag_dim: u32 = 4;
pub const flag_cursor_bar: u32 = 8; // vertical bar cursor; fill left edge, discard rest
pub const flag_cursor_underline: u32 = 16; // underline cursor; fill bottom, discard rest

/// What Zig hands the shim each frame. Matches `FrameData` read in shim.m.
pub const FrameData = extern struct {
    instances: [*]const CellInstance,
    count: u32,
    cell_w: f32,
    cell_h: f32,
    pad_x: f32,
    pad_y: f32,
    cell_uv: [2]f32,
    bar_h: f32, // title-bar height in device pixels
    bg: [3]f32, // canvas clear color
    bar_color: [3]f32,
    sep_color: [3]f32,
    dividers: [*]const f32, // flat x,y,w,h per pane divider (device px)
    divider_count: u32,
    // Command-palette overlay. `overlay` is a flat list of colored rects
    // (x,y,w,h,r,g,b per rect) drawn over the terminal but under the palette
    // text. `palette_text_count` glyph instances live in `instances` right
    // after the first `count`, drawn in a second pass on top of the overlay.
    overlay: [*]const f32,
    overlay_count: u32,
    palette_text_count: u32,
    // Glyphs newly assigned a cache slot this frame; the shim rasterizes each
    // `cp` into its `slot` before drawing. Empty once the cache is warm.
    pending: [*]const @import("atlas.zig").PendingGlyph,
    pending_count: u32,
};
