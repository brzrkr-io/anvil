// Renders a macOS .iconset from a square source PNG.
// Each size: source scaled to the Apple icon-grid body (~80.5% of the tile),
// centered on a transparent canvas, clipped to a rounded rect so the source's
// square corners become transparent — the native macOS icon shape.
// Usage: swift gen-iconset.swift <source.png> <out.iconset-dir>
import AppKit

let args = CommandLine.arguments
guard args.count == 3 else { FileHandle.standardError.write("usage: gen-iconset.swift <src.png> <outdir>\n".data(using: .utf8)!); exit(2) }
let srcPath = args[1]
let outDir = args[2]

guard let src = NSImage(contentsOfFile: srcPath) else { FileHandle.standardError.write("cannot load \(srcPath)\n".data(using: .utf8)!); exit(1) }

func render(_ size: Int) -> Data {
    let s = CGFloat(size)
    let body = (s * 824.0 / 1024.0).rounded()
    let off = ((s - body) / 2).rounded()
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = ctx
    ctx.imageInterpolation = .high
    let rect = NSRect(x: off, y: off, width: body, height: body)
    let radius = body * 0.2237
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).addClip()
    src.draw(in: rect, from: .zero, operation: .copy, fraction: 1.0)
    ctx.flushGraphics()
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

// (filename, pixel size)
let targets: [(String, Int)] = [
    ("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024),
]

let fm = FileManager.default
try? fm.createDirectory(atPath: outDir, withIntermediateDirectories: true)
for (name, size) in targets {
    let data = render(size)
    try! data.write(to: URL(fileURLWithPath: outDir).appendingPathComponent(name))
}
// Also emit a standalone 512 for runtime dock-icon embedding.
try! render(512).write(to: URL(fileURLWithPath: outDir).appendingPathComponent("app-icon-512.png"))
