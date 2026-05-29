#!/usr/bin/env bash
# Download the pinned Zig toolchain into .zig/ (gitignored).
# Run once per checkout; build with .zig/zig build.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$repo_root/tools/zig-version")"
dest="$repo_root/.zig"

# Map host to Zig's release naming.
case "$(uname -m)" in
    arm64 | aarch64) arch="aarch64" ;;
    x86_64) arch="x86_64" ;;
    *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
esac
case "$(uname -s)" in
    Darwin) os="macos" ;;
    Linux) os="linux" ;;
    *) echo "unsupported os: $(uname -s)" >&2; exit 1 ;;
esac

# Pinned SHA-256 of each official tarball for this version.
sha_aarch64_macos="b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489"
sha_x86_64_macos="0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7"
sha_aarch64_linux="ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17"
sha_x86_64_linux="70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00"
eval "expected_sha=\$sha_${arch}_${os}"

name="zig-${arch}-${os}-${version}"
url="https://ziglang.org/download/${version}/${name}.tar.xz"

if [ -x "$dest/zig" ] && [ "$("$dest/zig" version 2>/dev/null)" = "$version" ]; then
    echo "zig $version already present in .zig/"
    exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $url"
curl -fSL "$url" -o "$tmp/zig.tar.xz"

echo "verifying checksum"
if command -v sha256sum >/dev/null 2>&1; then
    actual_sha="$(sha256sum "$tmp/zig.tar.xz" | cut -d' ' -f1)"
else
    actual_sha="$(shasum -a 256 "$tmp/zig.tar.xz" | cut -d' ' -f1)"
fi
if [ "$actual_sha" != "$expected_sha" ]; then
    echo "checksum mismatch for $name" >&2
    echo "  expected $expected_sha" >&2
    echo "  actual   $actual_sha" >&2
    exit 1
fi

echo "extracting to .zig/"
tar -xJf "$tmp/zig.tar.xz" -C "$tmp"
rm -rf "$dest"
mv "$tmp/$name" "$dest"

echo "zig $("$dest/zig" version) ready at .zig/zig"
