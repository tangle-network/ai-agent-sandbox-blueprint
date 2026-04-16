#!/usr/bin/env bash
# Run all sandbox-runtime benchmarks and aggregate into a run manifest.
#
# Usage:
#   ./scripts/run-benches.sh                 # full run (~2-5 min per bench)
#   ./scripts/run-benches.sh --quick         # fast mode for CI / dev feedback
#   ./scripts/run-benches.sh --bench auth    # run a single bench only
#
# Outputs:
#   bench-results/latest.json     full manifest for this run
#   bench-results/runs.jsonl      append-only log of every run
#   target/criterion/             raw Criterion output (HTML, JSON)

set -euo pipefail

cd "$(dirname "$0")/.."

QUICK=false
BENCH=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) QUICK=true; shift ;;
        --bench) BENCH="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

# In quick mode, shorten Criterion's measurement time so CI jobs complete fast.
# Criterion's default is 5s measurement + 3s warmup per bench; we drop to 1s
# + 0.5s for regression detection on every PR.
CRIT_ARGS=""
if [[ "$QUICK" == "true" ]]; then
    CRIT_ARGS="--warm-up-time 1 --measurement-time 3 --sample-size 20"
fi

if [[ -n "$BENCH" ]]; then
    echo "[bench] running bench: $BENCH (quick=$QUICK)"
    cargo bench -p sandbox-runtime --features test-utils --bench "$BENCH" -- $CRIT_ARGS
else
    echo "[bench] running all benches (quick=$QUICK)"
    for bench in auth_bench session_auth_bench rate_limit_bench \
                 circuit_breaker_bench scoped_session_bench util_bench \
                 http_bench store_bench crypto_bench; do
        echo
        echo "============================================================"
        echo "  $bench"
        echo "============================================================"
        cargo bench -p sandbox-runtime --features test-utils --bench "$bench" -- $CRIT_ARGS
    done
fi

echo
echo "[bench] aggregating with bench-harness collect ..."
cargo run -p bench-harness --release -- collect \
    --workspace . \
    --output bench-results/latest.json \
    --jsonl bench-results/runs.jsonl

echo
echo "[bench] done. results in bench-results/latest.json"
