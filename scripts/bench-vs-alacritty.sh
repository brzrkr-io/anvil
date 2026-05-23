#!/usr/bin/env bash
# bench-vs-alacritty.sh — head-to-head vtebench runner: Anvil vs Alacritty.
#
# Usage:
#   ./scripts/bench-vs-alacritty.sh
#
# Prerequisites:
#   vtebench   — cargo install vtebench        (Alacritty's throughput suite)
#   alacritty  — brew install alacritty        (reference terminal)
#   anvil      — built in release mode by this script
#
# vtebench integration note:
#   vtebench drives terminals via their PTY in a headless subprocess mode.
#   Run `vtebench --help` to see the full option set. The flags used here are:
#     -c <cols> -r <rows>   terminal dimensions
#     -b <bytes>            bytes of output to generate per workload
#     <binary>              the terminal binary to wrap (must accept -e flag)
#
#   Anvil is a GUI app (not a pty-wrapping binary like alacritty), so it cannot
#   be driven by vtebench in the same way as Alacritty today. The script times
#   Alacritty automatically; for Anvil it prints the manual procedure.
#
#   Manual Anvil procedure (until a headless PTY mode is added):
#     1. Open Anvil.
#     2. In an Anvil terminal tab, run:
#          vtebench dense_cells | pv -ab > /dev/null
#        Record the avg throughput in MB/s; convert to ms for a fixed byte count
#        using: ms = (bytes / (MB_s * 1e6)) * 1000
#     3. Repeat for each workload below and fill in the "Anvil" column.

set -euo pipefail

# ── Dependency checks ─────────────────────────────────────────────────────────

if ! command -v vtebench &>/dev/null; then
    echo "ERROR: vtebench not found."
    echo "  Install:  cargo install vtebench"
    echo "  (or check https://github.com/alacritty/vtebench for other options)"
    exit 1
fi

if ! command -v alacritty &>/dev/null; then
    echo "ERROR: alacritty not found."
    echo "  Install:  brew install alacritty"
    exit 1
fi

# ── Build Anvil in release mode ───────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "Building Anvil (release)..."
cargo build -p anvil --release --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1 | tail -5

ANVIL_BIN="$REPO_ROOT/target/release/anvil"
if [[ ! -f "$ANVIL_BIN" ]]; then
    echo "ERROR: anvil binary not found at $ANVIL_BIN after build."
    exit 1
fi

# ── vtebench workloads ────────────────────────────────────────────────────────

WORKLOADS=(
    "dense_cells"
    "light_cells"
    "scrolling"
    "scrolling_in_region"
    "unicode"
)

COLS=80
ROWS=24
BYTES=10485760   # 10 MiB per workload

# ── Run against Alacritty ─────────────────────────────────────────────────────

echo ""
echo "Running vtebench against Alacritty..."
declare -A ALACRITTY_MS

for workload in "${WORKLOADS[@]}"; do
    # vtebench <workload> -c <cols> -r <rows> -b <bytes> -- <terminal> [args]
    # The terminal binary must accept standard Unix exec conventions.
    # Time the full vtebench run with `time`; capture elapsed ms.
    START_NS=$(date +%s%N)
    vtebench "$workload" -c "$COLS" -r "$ROWS" -b "$BYTES" -- alacritty -e cat 2>/dev/null || true
    END_NS=$(date +%s%N)
    ELAPSED_MS=$(( (END_NS - START_NS) / 1000000 ))
    ALACRITTY_MS[$workload]=$ELAPSED_MS
    echo "  alacritty / $workload: ${ELAPSED_MS} ms"
done

# ── Anvil: manual procedure ───────────────────────────────────────────────────

echo ""
echo "========================================================================"
echo "Anvil is a GUI app; vtebench cannot drive it headlessly today."
echo ""
echo "Manual Anvil measurement procedure:"
echo "  1. Launch Anvil:  $ANVIL_BIN"
echo "  2. In an Anvil terminal tab, run each command below and note the time:"
echo ""
for workload in "${WORKLOADS[@]}"; do
    echo "     $workload:"
    echo "       vtebench $workload -c $COLS -r $ROWS -b $BYTES | pv -ab > /dev/null"
    echo "       (record elapsed time in ms and enter it in docs/perf.md)"
done
echo ""
echo "  3. Update the 'Current Numbers' table in docs/perf.md."
echo "========================================================================"
echo ""

# ── Side-by-side summary table (Alacritty only until Anvil is measured) ──────

echo "Workload              Alacritty   Anvil       Delta"
echo "--------------------  ----------  ----------  -------"
for workload in "${WORKLOADS[@]}"; do
    ALC_MS=${ALACRITTY_MS[$workload]}
    printf "%-22s  %-10s  %-10s  %s\n" \
        "$workload" \
        "${ALC_MS} ms" \
        "(manual)" \
        "—"
done

echo ""
echo "To populate the Anvil column: follow the manual procedure above."
echo "To track results over time: append to the 'Current Numbers' table in docs/perf.md."
