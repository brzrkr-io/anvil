# Caldera Console

A native Zig rebuild of Caldera Console — a unified control plane for software
and devops work. See `docs/product/console-rebuild-plan.md` for the full plan.

Status: **M0** — Zig scaffold (native window + Metal clear).

## Build

```sh
zig build run     # build and launch
zig build test    # run unit tests
```

Requires Zig (see `minimum_zig_version` in `build.zig.zon`) and macOS with
Xcode Command Line Tools.
