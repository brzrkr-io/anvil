const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const mod = b.addModule("anvil", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .link_libc = true,
    });

    const exe = b.addExecutable(.{
        .name = "anvil",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
            .imports = &.{.{ .name = "anvil", .module = mod }},
        }),
    });
    exe.root_module.addAnonymousImport("font_ttf", .{
        .root_source_file = b.path("assets/BlexMonoNerdFontMono-Regular.ttf"),
    });
    exe.root_module.addAnonymousImport("font_ttf_bold", .{
        .root_source_file = b.path("assets/BlexMonoNerdFontMono-Bold.ttf"),
    });
    exe.root_module.addAnonymousImport("sans_ttf", .{
        .root_source_file = b.path("assets/IBMPlexSans-Regular.otf"),
    });
    exe.root_module.addAnonymousImport("app_icon_png", .{
        .root_source_file = b.path("assets/app-icon.png"),
    });
    exe.root_module.addCSourceFile(.{
        .file = b.path("src/platform/shim.m"),
        .flags = &.{"-fobjc-arc"},
    });
    exe.root_module.linkFramework("Cocoa", .{});
    exe.root_module.linkFramework("QuartzCore", .{});
    exe.root_module.linkFramework("Metal", .{});
    exe.root_module.linkFramework("CoreText", .{});
    exe.root_module.linkFramework("CoreGraphics", .{});
    exe.root_module.linkFramework("ImageIO", .{});
    exe.root_module.linkFramework("UniformTypeIdentifiers", .{});
    exe.root_module.linkFramework("UserNotifications", .{});
    exe.root_module.linkFramework("WebKit", .{});
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);
    const run_step = b.step("run", "Run the app");
    run_step.dependOn(&run_cmd.step);

    // macOS .app bundle: zig-out/Anvil.app with Info.plist + AppIcon.icns.
    const bundle_exe = b.addInstallArtifact(exe, .{
        .dest_dir = .{ .override = .{ .custom = "Anvil.app/Contents/MacOS" } },
    });
    const bundle_icns = b.addInstallFileWithDir(
        b.path("assets/AppIcon.icns"),
        .{ .custom = "Anvil.app/Contents/Resources" },
        "AppIcon.icns",
    );
    const plist = b.addWriteFiles();
    const plist_path = plist.add("Info.plist", info_plist);
    const bundle_plist = b.addInstallFileWithDir(
        plist_path,
        .{ .custom = "Anvil.app/Contents" },
        "Info.plist",
    );
    const bundle_step = b.step("bundle", "Build the macOS Anvil.app bundle");
    bundle_step.dependOn(&bundle_exe.step);
    bundle_step.dependOn(&bundle_icns.step);
    bundle_step.dependOn(&bundle_plist.step);

    const mod_tests = b.addTest(.{ .root_module = mod });
    const exe_tests = b.addTest(.{ .root_module = exe.root_module });
    const test_step = b.step("test", "Run tests");
    test_step.dependOn(&b.addRunArtifact(mod_tests).step);
    test_step.dependOn(&b.addRunArtifact(exe_tests).step);
}

const info_plist =
    \\<?xml version="1.0" encoding="UTF-8"?>
    \\<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    \\<plist version="1.0">
    \\<dict>
    \\  <key>CFBundleName</key><string>Anvil</string>
    \\  <key>CFBundleDisplayName</key><string>Anvil</string>
    \\  <key>CFBundleExecutable</key><string>anvil</string>
    \\  <key>CFBundleIdentifier</key><string>io.brzrkr.anvil</string>
    \\  <key>CFBundleIconFile</key><string>AppIcon</string>
    \\  <key>CFBundlePackageType</key><string>APPL</string>
    \\  <key>CFBundleShortVersionString</key><string>0.1.0</string>
    \\  <key>CFBundleVersion</key><string>0.1.0</string>
    \\  <key>LSMinimumSystemVersion</key><string>13.0</string>
    \\  <key>NSHighResolutionCapable</key><true/>
    \\  <key>NSPrincipalClass</key><string>NSApplication</string>
    \\</dict>
    \\</plist>
    \\
;
