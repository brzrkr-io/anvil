const std = @import("std");
const objc = @import("objc");

pub fn main() void {
    const has_nsobject = objc.getClass("NSObject") != null;
    std.debug.print("caldera-console: build OK (objc runtime: {})\n", .{has_nsobject});
}
