const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const objc_dep = b.dependency("zig_objc", .{
        .target = target,
        .optimize = optimize,
    });

    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
        .imports = &.{
            .{ .name = "objc", .module = objc_dep.module("objc") },
        },
    });
    exe_mod.linkFramework("AppKit", .{});
    exe_mod.linkFramework("Metal", .{});
    exe_mod.linkFramework("QuartzCore", .{});
    exe_mod.linkFramework("CoreText", .{});
    exe_mod.linkFramework("CoreGraphics", .{});
    exe_mod.linkFramework("CoreFoundation", .{});

    const exe = b.addExecutable(.{
        .name = "caldera-console",
        .root_module = exe_mod,
    });
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);
    const run_step = b.step("run", "Run the app");
    run_step.dependOn(&run_cmd.step);

    const exe_tests = b.addTest(.{ .root_module = exe_mod });
    const run_exe_tests = b.addRunArtifact(exe_tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_exe_tests.step);

    // Coverage build: kcov deadlocks tracing the child processes the pty tests
    // spawn on macOS, so the report is built from a root that imports every
    // module except pty/. pty is still covered by `zig build test`.
    const cov_mod = b.createModule(.{
        .root_source_file = b.path("src/coverage_root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
        .imports = &.{
            .{ .name = "objc", .module = objc_dep.module("objc") },
        },
    });
    cov_mod.linkFramework("AppKit", .{});
    cov_mod.linkFramework("Metal", .{});
    cov_mod.linkFramework("QuartzCore", .{});
    cov_mod.linkFramework("CoreText", .{});
    cov_mod.linkFramework("CoreGraphics", .{});
    cov_mod.linkFramework("CoreFoundation", .{});
    const cov_tests = b.addTest(.{ .root_module = cov_mod });

    // Run the coverage test binary under kcov and emit a line-coverage report.
    const coverage = b.addSystemCommand(&.{ "kcov", "--clean", "--include-pattern=src/" });
    const coverage_out = coverage.addOutputDirectoryArg("coverage");
    coverage.addArtifactArg(cov_tests);
    const install_coverage = b.addInstallDirectory(.{
        .source_dir = coverage_out,
        .install_dir = .prefix,
        .install_subdir = "coverage",
    });
    const coverage_step = b.step("coverage", "Run tests under kcov and emit a coverage report");
    coverage_step.dependOn(&install_coverage.step);
}
