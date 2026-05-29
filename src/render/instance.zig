/// Per-cell GPU instance. Layout must match `Instance` in shaders.metal.
pub const CellInstance = extern struct {
    col: f32,
    row: f32,
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
};
