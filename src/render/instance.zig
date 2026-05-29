/// Per-cell GPU instance. Layout must match `Instance` in shaders.metal.
/// `x`/`y` are the cell's top-left in device pixels (pane origin baked in).
pub const CellInstance = extern struct {
    x: f32,
    y: f32,
    fg: [4]f32,
    bg: [4]f32,
    uv: [2]f32,
};

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
};
