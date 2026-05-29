extern fn anvil_run() void;
extern fn anvil_dump(path: [*:0]const u8, w: u32, h: u32) void;

pub fn run() void {
    anvil_run();
}

pub fn dump(path: [*:0]const u8, w: u32, h: u32) void {
    anvil_dump(path, w, h);
}
