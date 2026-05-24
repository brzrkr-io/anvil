#!/usr/bin/env bash
# Build + launch Anvil. Always builds anvil-prompt alongside anvil so the
# shell-prompt binary the spawned login shell calls is current.
#
# Usage:
#   scripts/run.sh                  — debug build, launch
#   scripts/run.sh --release        — release build, launch
set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE_FLAGS=()
RUN_PROFILE_FLAGS=()
if [[ "${1:-}" == "--release" ]]; then
  PROFILE_FLAGS+=("--release")
  RUN_PROFILE_FLAGS+=("--release")
fi

# Build both binaries first so the prompt binary is current before launch.
cargo build "${PROFILE_FLAGS[@]}" -p anvil -p anvil-prompt

# Launch anvil (no-op rebuild if up to date).
exec cargo run "${RUN_PROFILE_FLAGS[@]}" -p anvil --bin anvil
