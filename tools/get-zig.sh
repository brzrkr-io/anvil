#!/usr/bin/env bash
# Download the pinned Zig toolchain (compiler + zls) into .zig/ (gitignored)
# and write a gitignored zls.json that points zls at the vendored compiler.
# Run once per checkout; build with .zig/zig build.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$repo_root/tools/zig-version")"
dest="$repo_root/.zig"

# Map host to the release naming both projects share: <arch>-<os>.
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
triple="${arch}-${os}"

# Pinned SHA-256 of each official tarball for this version.
zig_sha_aarch64_macos="b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489"
zig_sha_x86_64_macos="0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7"
zig_sha_aarch64_linux="ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17"
zig_sha_x86_64_linux="70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00"
zls_sha_aarch64_macos="b93ec549f8558a7e85984a840e9276d274f1059b54ade4254296ef4982958359"
zls_sha_x86_64_macos="49f716ea96c1aadaecaa5d9c0a50874cbcf443dc42b825f1e7ee35499ad3eb96"
zls_sha_aarch64_linux="430cd293d201eb70ae2519dbc96c854bf8791b8df7fc9392e8d2dc9680a2bed7"
zls_sha_x86_64_linux="ded6d562a0b86ee878b1ddf70ffab2797ce3cdca3b02d6077548f9d56dff96b6"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# fetch <url> <expected-sha> <out-tar>
fetch() {
    echo "downloading $1"
    curl -fSL "$1" -o "$3"
    local actual; actual="$(sha256 "$3")"
    if [ "$actual" != "$2" ]; then
        echo "checksum mismatch for $1" >&2
        echo "  expected $2" >&2
        echo "  actual   $actual" >&2
        exit 1
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$dest"

# --- Zig compiler ---
if [ -x "$dest/zig" ] && [ "$("$dest/zig" version 2>/dev/null)" = "$version" ]; then
    echo "zig $version already present in .zig/"
else
    eval "zig_sha=\$zig_sha_${arch}_${os}"
    zig_name="zig-${triple}-${version}"
    fetch "https://ziglang.org/download/${version}/${zig_name}.tar.xz" "$zig_sha" "$tmp/zig.tar.xz"
    tar -xJf "$tmp/zig.tar.xz" -C "$tmp"
    rm -rf "$dest/zig-dist"
    mv "$tmp/$zig_name" "$dest/zig-dist"
    ln -sf zig-dist/zig "$dest/zig"
    echo "zig $("$dest/zig" version) ready at .zig/zig"
fi

# --- zls (version-locked to the compiler) ---
if [ -x "$dest/zls" ] && [ "$("$dest/zls" --version 2>/dev/null)" = "$version" ]; then
    echo "zls $version already present in .zig/"
else
    eval "zls_sha=\$zls_sha_${arch}_${os}"
    fetch "https://github.com/zigtools/zls/releases/download/${version}/zls-${triple}.tar.xz" "$zls_sha" "$tmp/zls.tar.xz"
    mkdir -p "$tmp/zls-dist"
    tar -xJf "$tmp/zls.tar.xz" -C "$tmp/zls-dist"
    install "$tmp/zls-dist/zls" "$dest/zls"
    echo "zls $("$dest/zls" --version) ready at .zig/zls"
fi

# --- zls.json: force zls to analyze with the vendored compiler ---
# Absolute path so it resolves regardless of the editor's working directory.
# Machine-specific, so it is gitignored and regenerated here.
cat > "$repo_root/zls.json" <<JSON
{
    "zig_exe_path": "$dest/zig"
}
JSON
echo "wrote zls.json -> zig_exe_path=$dest/zig"
