pub const Color = union(enum) {
    default,
    indexed: u8,
    rgb: struct { r: u8, g: u8, b: u8 },
};

pub const Attrs = packed struct {
    bold: bool = false,
    underline: bool = false,
    reverse: bool = false,
    italic: bool = false,
    dim: bool = false,
    strike: bool = false,
    blink: bool = false,
};

pub const Cell = struct {
    cp: u21 = ' ',
    fg: Color = .default,
    bg: Color = .default,
    attrs: Attrs = .{},

    pub const blank: Cell = .{};
};
