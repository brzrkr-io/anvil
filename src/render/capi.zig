//! Hand-written extern declarations for the Apple C APIs the renderer uses:
//! CoreFoundation, CoreGraphics, CoreText. Declared by hand (not @cImport) to
//! keep the surface tiny and dodge translate-c fragility on Zig 0.16.

pub const CGFloat = f64;
pub const CGGlyph = u16;
pub const CFIndex = c_long;

/// Opaque CoreFoundation/CoreGraphics/CoreText reference.
pub const Ref = ?*anyopaque;

pub const CGPoint = extern struct { x: CGFloat = 0, y: CGFloat = 0 };
pub const CGSize = extern struct { width: CGFloat = 0, height: CGFloat = 0 };
pub const CGRect = extern struct { origin: CGPoint = .{}, size: CGSize = .{} };
pub const CGAffineTransform = extern struct {
    a: CGFloat,
    b: CGFloat,
    c: CGFloat,
    d: CGFloat,
    tx: CGFloat,
    ty: CGFloat,
};
pub const identity: CGAffineTransform = .{ .a = 1, .b = 0, .c = 0, .d = 1, .tx = 0, .ty = 0 };

// --- CoreFoundation ---
pub const kCFStringEncodingUTF8: u32 = 0x08000100;
pub extern fn CFRelease(Ref) void;
pub extern fn CFStringCreateWithCString(alloc: Ref, cstr: [*:0]const u8, encoding: u32) Ref;

// --- CoreGraphics ---
// BGRA8, premultiplied alpha, little-endian 32-bit words — matches Metal
// MTLPixelFormatBGRA8Unorm.
pub const kCGImageAlphaPremultipliedFirst: u32 = 2;
pub const kCGBitmapByteOrder32Little: u32 = 2 << 12;
pub const bgra8_bitmap_info: u32 = kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little;

pub extern fn CGColorSpaceCreateDeviceRGB() Ref;
pub extern fn CGColorSpaceRelease(Ref) void;
pub extern fn CGBitmapContextCreate(
    data: ?*anyopaque,
    width: usize,
    height: usize,
    bits_per_component: usize,
    bytes_per_row: usize,
    space: Ref,
    bitmap_info: u32,
) Ref;
pub extern fn CGContextRelease(Ref) void;
pub extern fn CGContextSetRGBFillColor(ctx: Ref, r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat) void;
pub extern fn CGContextFillRect(ctx: Ref, rect: CGRect) void;
pub extern fn CGContextClearRect(ctx: Ref, rect: CGRect) void;
pub extern fn CGContextSetShouldAntialias(ctx: Ref, on: bool) void;
pub extern fn CGContextSetTextMatrix(ctx: Ref, t: CGAffineTransform) void;

// --- CoreText ---
pub const kCTFontOrientationDefault: u32 = 0;
pub extern fn CTFontCreateWithName(name: Ref, size: CGFloat, matrix: ?*const CGAffineTransform) Ref;
pub extern fn CTFontGetAscent(font: Ref) CGFloat;
pub extern fn CTFontGetDescent(font: Ref) CGFloat;
pub extern fn CTFontGetLeading(font: Ref) CGFloat;
pub extern fn CTFontGetGlyphsForCharacters(
    font: Ref,
    chars: [*]const u16,
    glyphs: [*]CGGlyph,
    count: CFIndex,
) bool;
pub extern fn CTFontGetAdvancesForGlyphs(
    font: Ref,
    orientation: u32,
    glyphs: [*]const CGGlyph,
    advances: ?[*]CGSize,
    count: CFIndex,
) CGFloat;
pub extern fn CTFontDrawGlyphs(
    font: Ref,
    glyphs: [*]const CGGlyph,
    positions: [*]const CGPoint,
    count: usize,
    context: Ref,
) void;
