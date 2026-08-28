#!/usr/bin/env bash
# Verify configured blueprint IDs and BSM addresses against the selected chain.
#
# Arguments:
#   1. RPC URL
#   2. Tangle core address
#   3. Expected decimal chain ID
#   4. File containing binary:id:bsmAddress rows
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: verify-blueprint-release-registry.sh <rpc-url> <tangle-core> <chain-id> <details-file>" >&2
  exit 2
fi

RPC_URL="$1"
TANGLE_CORE="$2"
EXPECTED_CHAIN_ID="$3"
DETAILS_FILE="$4"

[[ -f "$DETAILS_FILE" ]] || { echo "ERROR: registry details missing at $DETAILS_FILE" >&2; exit 1; }
[[ "$TANGLE_CORE" =~ ^0x[0-9a-fA-F]{40}$ ]] || { echo "ERROR: invalid tangle core address" >&2; exit 1; }
[[ "$EXPECTED_CHAIN_ID" =~ ^[0-9]+$ ]] || { echo "ERROR: invalid expected chain ID" >&2; exit 1; }
awk -F: 'NF != 3 || $1 == "" || $2 == "" || $3 == "" { exit 1 }' "$DETAILS_FILE" \
  || { echo "ERROR: malformed registry details" >&2; exit 1; }

ACTUAL_CHAIN_ID="$(cast chain-id --rpc-url "$RPC_URL" | tr -d '[:space:]')"
[[ "$ACTUAL_CHAIN_ID" == "$EXPECTED_CHAIN_ID" ]] || {
  echo "ERROR: RPC chain ID $ACTUAL_CHAIN_ID does not match $EXPECTED_CHAIN_ID" >&2
  exit 1
}

found=0
while IFS=: read -r bin id expected_bsm; do
  [[ -n "$bin" ]] || continue
  [[ "$id" =~ ^(0|[1-9][0-9]*)$ ]] || { echo "ERROR: invalid ID for $bin" >&2; exit 1; }
  [[ "$expected_bsm" =~ ^0x[0-9a-fA-F]{40}$ ]] || { echo "ERROR: invalid BSM for $bin" >&2; exit 1; }
  [[ "${expected_bsm,,}" != "0x0000000000000000000000000000000000000000" ]] \
    || { echo "ERROR: zero BSM for $bin" >&2; exit 1; }

  actual_bsm="$(cast call "$TANGLE_CORE" \
    'getBlueprint(uint64)((address,address,uint64,uint32,uint8,uint8,bool))' \
    "$id" --rpc-url "$RPC_URL" \
    | sed -En 's/^\(0x[0-9a-fA-F]{40},[[:space:]]*(0x[0-9a-fA-F]{40}).*/\1/p')"
  [[ -n "$actual_bsm" ]] || {
    echo "ERROR: cannot decode the BSM for $bin (blueprint $id)" >&2
    exit 1
  }
  [[ "${actual_bsm,,}" == "${expected_bsm,,}" ]] || {
    echo "ERROR: blueprint $id BSM $actual_bsm does not match configured $expected_bsm for $bin" >&2
    exit 1
  }
  found=1
  echo "verified $bin -> blueprint $id -> BSM $actual_bsm"
done < "$DETAILS_FILE"
[[ "$found" -eq 1 ]] || { echo "ERROR: registry details are empty" >&2; exit 1; }
