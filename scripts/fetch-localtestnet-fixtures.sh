#!/usr/bin/env bash
set -euo pipefail

# Downloads tnt-core-fixtures from crates.io and places the Anvil state
# snapshot files where blueprint-chain-setup-anvil expects them.
#
# Usage:
#   scripts/fetch-localtestnet-fixtures.sh           # idempotent, skips if present
#   scripts/fetch-localtestnet-fixtures.sh --force    # re-download even if present

FORCE=0
for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

CRATE_NAME="${TNT_FIXTURES_CRATE:-tnt-core-fixtures}"
VERSION="${TNT_FIXTURES_VERSION:-}"

# Locate the blueprint-chain-setup-anvil package inside cargo's git checkout
# so that env!("CARGO_MANIFEST_DIR") resolves correctly at build time.
CHAIN_SETUP_DIR=$(cargo metadata --format-version=1 2>/dev/null | \
  python3 -c "
import json, sys
pkgs = json.load(sys.stdin)['packages']
print(next(p['manifest_path'] for p in pkgs if p['name'] == 'blueprint-chain-setup-anvil'))
" | xargs dirname)

OUT_DIR="${CHAIN_SETUP_DIR}/snapshots"

# Skip if fixtures already exist (unless --force)
if [[ "$FORCE" -eq 0 ]] && \
   [[ -f "${OUT_DIR}/localtestnet-state.json" ]] && \
   [[ -f "${OUT_DIR}/localtestnet-broadcast.json" ]]; then
  echo "Fixtures already present in ${OUT_DIR} (use --force to re-download)"
  exit 0
fi

# Auto-detect latest version from crates.io
if [[ -z "$VERSION" ]]; then
  VERSION="$(curl -sS -H "User-Agent: ai-agent-sandbox-blueprint" \
    "https://crates.io/api/v1/crates/${CRATE_NAME}" | \
    python3 -c "import json,sys; print(json.load(sys.stdin)['crate']['newest_version'])")"
fi

if [[ -z "$VERSION" ]]; then
  echo "error: could not detect latest ${CRATE_NAME} version; set TNT_FIXTURES_VERSION" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d -t tnt-fixtures.XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

TARBALL="${TMP_DIR}/${CRATE_NAME}-${VERSION}.crate"

echo "Fetching ${CRATE_NAME} ${VERSION} from crates.io..."
curl -sSL -H "User-Agent: ai-agent-sandbox-blueprint" \
  "https://crates.io/api/v1/crates/${CRATE_NAME}/${VERSION}/download" \
  -o "$TARBALL"
tar -xzf "$TARBALL" -C "$TMP_DIR"

SRC_DIR="${TMP_DIR}/${CRATE_NAME}-${VERSION}/fixtures"
STATE_SRC="${SRC_DIR}/localtestnet-state.json"
BROADCAST_SRC="${SRC_DIR}/localtestnet-broadcast.json"

if [[ ! -f "$STATE_SRC" || ! -f "$BROADCAST_SRC" ]]; then
  echo "error: fixture files not found in ${CRATE_NAME} ${VERSION}" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
cp "$STATE_SRC" "$OUT_DIR/localtestnet-state.json"
cp "$BROADCAST_SRC" "$OUT_DIR/localtestnet-broadcast.json"

echo "Wrote fixtures to ${OUT_DIR}"
