#!/usr/bin/env bash
# Run mini-vec-engine under Linux perf to capture hardware counters.
#
# Usage:
#   ./scripts/bench-perf.sh            # Full report
#   ./scripts/bench-perf.sh record     # Record + annotate (needs perf record)
#
# Counters: cycles, instructions, branches, branch-misses,
#           cache-references, cache-misses, L1-dcache-loads, L1-dcache-load-misses

set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Building release binary ==="
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release 2>&1 | tail -1

BIN="./target/release/mini-vec-engine"

echo ""
echo "=== perf stat ==="
perf stat \
    -e cycles,instructions,branches,branch-misses \
    -e cache-references,cache-misses \
    -e L1-dcache-loads,L1-dcache-load-misses \
    "$BIN" 2>&1

if [[ "${1:-}" == "record" ]]; then
    echo ""
    echo "=== perf record + report ==="
    perf record -g "$BIN" 2>&1
    perf report --stdio 2>&1 | head -60
fi
