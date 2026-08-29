#!/usr/bin/env bash
# Publish the on-chain cold-start sources for every blueprint in this repo so a
# STOCK blueprint-manager fetches the released binary from GitHub Releases
# (sha256-verified) instead of falling back to a cargo source build.
#
# What this writes per blueprint (Types.BlueprintSource, tnt-core >= the
# 2026-05 facet upgrade that added `setBlueprintSources` + made
# `getBlueprintDefinition` reflect live sources):
#
#   kind    = Native (2)
#   native  = { fetcher: Http (2),
#               artifactUri: {"dist_url":   <release dist-manifest.json>,
#                             "archive_url":<release <bin>-<triple>.tar.xz>,
#                             "binaries":   []},
#               entrypoint: <binary name> }
#   binaries = [{ arch, os: Linux (1), name: <binary>, sha256: <EXTRACTED
#                 binary hash from the release's .bin.sha256 asset> }]
#
# One source per architecture. The manager's RemoteBinaryFetcher
# (crates/manager/src/sources/remote.rs in tangle-network/blueprint):
#   1. picks the source whose binaries[] matches the host os/arch,
#   2. downloads dist_url and requires an executable-zip artifact whose
#      executable asset name equals the binary name,
#   3. downloads archive_url, unpacks, and verifies the EXTRACTED binary's
#      sha256 against the on-chain bytes32 (fail-closed on mismatch).
# That is why `binaries[].sha256` must be the .bin.sha256 (extracted binary),
# NOT the tarball hash.
#
# Encoding mirrors tnt-core's canonical publish path
# (.github/workflows/publish-blueprint-binary.yml "Update blueprint cold-start
# sources" step). cargo-tangle has no sources-update command today; cast +
# setBlueprintSources is the supported tooling.
#
# Usage:
#   ./deploy/publish-blueprint-sources.sh v0.1.1                   # dry-run
#   BROADCAST=true BLUEPRINT_OWNER_PRIVATE_KEY=0x... \
#     ./deploy/publish-blueprint-sources.sh v0.1.1                 # send txs
#
# Options / env:
#   TAG (arg 1)         — release tag in this repo (required).
#   BROADCAST           — "true" to send transactions. Default: dry-run.
#   ONLY                — restrict to one binary name (e.g. ai-agent-sandbox-blueprint).
#   RPC_URL             — default https://sepolia.base.org.
#   TANGLE_CORE         — must match the vendored Base Sepolia manifest when set.
#   BLUEPRINT_REPO      — default tangle-network/ai-agent-sandbox-blueprint.
#   BLUEPRINT_OWNER_PRIVATE_KEY — owner key (required with BROADCAST=true).
#   SKIP_ARCHIVE_VERIFY — "1" to skip downloading + re-hashing the archives
#                         before broadcast (verification is ON by default).
#
# Idempotent: a blueprint whose on-chain sources already carry this tag's
# archive URLs and binary hashes is skipped.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

TAG="${1:?usage: publish-blueprint-sources.sh <tag> (e.g. v0.1.1)}"
BROADCAST="${BROADCAST:-false}"
ONLY="${ONLY:-}"
RPC_URL="${RPC_URL:-https://sepolia.base.org}"
BLUEPRINT_REPO="${BLUEPRINT_REPO:-tangle-network/ai-agent-sandbox-blueprint}"
SKIP_ARCHIVE_VERIFY="${SKIP_ARCHIVE_VERIFY:-0}"
CONFIG="$ROOT_DIR/deploy/manifests/base-sepolia/blueprints.json"
MANIFEST="$ROOT_DIR/deploy/manifests/base-sepolia/tnt-core.latest.json"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

[[ -f "$CONFIG" ]] || { echo "ERROR: release registry missing at $CONFIG" >&2; exit 1; }
[[ -f "$MANIFEST" ]] || { echo "ERROR: deployment manifest missing at $MANIFEST" >&2; exit 1; }

MANIFEST_NETWORK="$(jq -er '.network' "$MANIFEST")"
MANIFEST_CHAIN_ID="$(jq -er '.chainId' "$MANIFEST")"
MANIFEST_TANGLE_CORE="$(jq -er '.tangle' "$MANIFEST")"
TANGLE_CORE="${TANGLE_CORE:-$MANIFEST_TANGLE_CORE}"
REGISTRY_URL="https://raw.githubusercontent.com/tangle-network/tnt-core/main/deployments/base-sepolia/blueprints.tsv"

[[ "$MANIFEST_NETWORK" == "base-sepolia" ]] || {
  echo "ERROR: source publisher only supports the Base Sepolia manifest" >&2
  exit 1
}

case "$BROADCAST" in
  true|false) ;;
  *) echo "ERROR: BROADCAST must be true or false" >&2; exit 1 ;;
esac
case "$SKIP_ARCHIVE_VERIFY" in
  0|1) ;;
  *) echo "ERROR: SKIP_ARCHIVE_VERIFY must be 0 or 1" >&2; exit 1 ;;
esac
if [[ "$BROADCAST" == "true" && "$SKIP_ARCHIVE_VERIFY" == "1" ]]; then
  echo "ERROR: SKIP_ARCHIVE_VERIFY=1 is allowed only for dry runs" >&2
  exit 1
fi

TNT_CORE_REGISTRY="$WORK_DIR/tnt-core-blueprints.tsv"
curl -fsSL --retry 3 -o "$TNT_CORE_REGISTRY" "$REGISTRY_URL" \
  || { echo "ERROR: cannot read the tnt-core Base Sepolia registration ledger" >&2; exit 1; }

# The registry is checked against the deployment manifest before any release
# asset is downloaded or chain state is read. It owns every binary/id pair,
# including the registered TEE instance blueprint, and is checked against the
# current tnt-core registration ledger.
CONFIG_DETAILS_FILE="$WORK_DIR/config-details"
node "$ROOT_DIR/scripts/blueprint-release-config.mjs" \
  --file "$CONFIG" \
  --repository "$BLUEPRINT_REPO" \
  --registration-file "$TNT_CORE_REGISTRY" \
  --deployment-manifest "$MANIFEST" \
  --network "$MANIFEST_NETWORK" \
  --chain-id "$MANIFEST_CHAIN_ID" \
  --tangle-core "$TANGLE_CORE" \
  --format details > "$CONFIG_DETAILS_FILE"
CONFIG_ENTRIES="$(awk -F: '{print $1 ":" $2}' "$CONFIG_DETAILS_FILE")"

if [[ -n "$ONLY" ]]; then
  ONLY_FOUND=false
  while IFS=: read -r config_bin _ _; do
    [[ "$config_bin" == "$ONLY" ]] && ONLY_FOUND=true
  done <<< "$CONFIG_ENTRIES"
  [[ "$ONLY_FOUND" == true ]] || {
    echo "ERROR: ONLY=$ONLY is not present in the release registry" >&2
    exit 1
  }
fi

# arch discriminators from Types.BlueprintArchitecture (Amd64=5, Arm64=7).
TRIPLES=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu)
declare -A TRIPLE_ARCH=(
  [x86_64-unknown-linux-gnu]=5
  [aarch64-unknown-linux-gnu]=7
)

BASE_URL="https://github.com/${BLUEPRINT_REPO}/releases/download/${TAG}"
SOURCES_SIG='blueprintSources(uint64)((uint8,(string,string,string),(uint8,uint8,string,string),(uint8,string,string),(string,string,string),(uint8,uint8,string,bytes32)[])[])'
SET_SIG='setBlueprintSources(uint64,(uint8,(string,string,string),(uint8,uint8,string,string),(uint8,string,string),(string,string,string),(uint8,uint8,string,bytes32)[])[])'

echo "=== Publish blueprint cold-start sources ==="
echo "  Tag:         $TAG"
echo "  Repo:        $BLUEPRINT_REPO"
echo "  Network:     $MANIFEST_NETWORK"
echo "  Chain ID:    $MANIFEST_CHAIN_ID"
echo "  Tangle core: $TANGLE_CORE"
echo "  RPC:         $RPC_URL"
echo "  Broadcast:   $BROADCAST"
echo

"$ROOT_DIR/scripts/verify-blueprint-release-registry.sh" \
  "$RPC_URL" "$TANGLE_CORE" "$MANIFEST_CHAIN_ID" "$CONFIG_DETAILS_FILE"

# The manager refuses to download unless dist-manifest.json declares the
# binary as an executable-zip artifact with an executable asset of that name.
DIST_MANIFEST="$WORK_DIR/dist-manifest.json"
curl -fsSL --retry 3 -o "$DIST_MANIFEST" "$BASE_URL/dist-manifest.json" \
  || { echo "ERROR: $TAG has no dist-manifest.json release asset" >&2; exit 1; }

if [[ "$BROADCAST" == "true" ]]; then
  : "${BLUEPRINT_OWNER_PRIVATE_KEY:?ERROR: BLUEPRINT_OWNER_PRIVATE_KEY required with BROADCAST=true}"
  # cast >=1.7 does not read the signing key from ETH_PRIVATE_KEY; pass it
  # explicitly to the wallet/send calls instead.
  export ETH_PRIVATE_KEY="$BLUEPRINT_OWNER_PRIVATE_KEY"
  SENDER="$(cast wallet address --private-key "$ETH_PRIVATE_KEY")"
  echo "  Sender:      $SENDER"
  echo
fi

publish_one() {
  local bin="$1" id="$2"

  echo "--- $bin (blueprint $id) ---"

  jq -e --arg b "$bin" \
    '.artifacts[$b] | select(.kind == "executable-zip") | .assets[] | select(.kind == "executable" and .name == $b)' \
    "$DIST_MANIFEST" >/dev/null \
    || { echo "ERROR: dist-manifest.json does not declare executable $bin (manager would refuse the download)" >&2; return 1; }

  local sources="" expected_sources=() triple sha sha_file uri archive_url
  for triple in "${TRIPLES[@]}"; do
    sha_file="$WORK_DIR/${bin}-${triple}.bin.sha256"
    if ! curl -fsSL --retry 3 -o "$sha_file" "$BASE_URL/${bin}-${triple}.bin.sha256" 2>/dev/null; then
      echo "  $triple: no .bin.sha256 asset on $TAG — skipping arch"
      continue
    fi
    sha="0x$(tr -d '[:space:]' < "$sha_file")"
    [[ "${#sha}" -eq 66 ]] || { echo "ERROR: invalid sha256 in ${bin}-${triple}.bin.sha256" >&2; return 1; }

    archive_url="${BASE_URL}/${bin}-${triple}.tar.xz"

    if [[ "$SKIP_ARCHIVE_VERIFY" != "1" ]]; then
      # Fail-closed: prove the published .bin.sha256 matches the archive's
      # extracted binary BEFORE writing it on-chain. A wrong hash here bricks
      # every operator cold-start (manager rejects the verified download).
      local extract_dir="$WORK_DIR/verify-${bin}-${triple}"
      mkdir -p "$extract_dir"
      curl -fsSL --retry 3 -o "$extract_dir/a.tar.xz" "$archive_url" \
        || { echo "ERROR: missing release archive $archive_url" >&2; return 1; }
      tar -xJf "$extract_dir/a.tar.xz" -C "$extract_dir"
      [[ -f "$extract_dir/$bin" ]] || { echo "ERROR: archive $archive_url does not contain $bin" >&2; return 1; }
      local actual
      actual="0x$(sha256sum "$extract_dir/$bin" | awk '{print $1}')"
      [[ "$actual" == "$sha" ]] \
        || { echo "ERROR: $triple extracted-binary hash $actual != published $sha" >&2; return 1; }
      echo "  $triple: archive verified (extracted sha256 matches .bin.sha256)"
    fi

    uri="{\"dist_url\":\"${BASE_URL}/dist-manifest.json\",\"archive_url\":\"${archive_url}\",\"binaries\":[]}"
    local source="(2,(\"\",\"\",\"\"),(0,0,\"\",\"\"),(2,'${uri}',\"${bin}\"),(\"\",\"\",\"\"),[(${TRIPLE_ARCH[$triple]},1,\"${bin}\",${sha})])"
    sources="${sources:+${sources},}${source}"
    local escaped_uri
    escaped_uri="${uri//\"/\\\"}"
    expected_sources+=("(2,(\"\",\"\",\"\"),(0,0,\"\",\"\"),(2,\"${escaped_uri}\",\"${bin}\"),(\"\",\"\",\"\"),[(${TRIPLE_ARCH[$triple]},1,\"${bin}\",${sha})])")
    echo "  $triple: sha256 ${sha#0x}"
  done

  [[ -n "$sources" ]] || { echo "ERROR: no linux .bin.sha256 assets found for ${bin}@${TAG}" >&2; return 1; }
  sources="[${sources}]"

  # Idempotency: skip only when every complete source tuple already contains
  # this tag's URL, binary name, architecture, and extracted-binary hash.
  local current
  current="$(cast call "$TANGLE_CORE" "$SOURCES_SIG" "$id" --rpc-url "$RPC_URL")"
  local compact_current
  compact_current="$(tr -d '[:space:]' <<< "$current")"
  local up_to_date=1 expected_source
  for expected_source in "${expected_sources[@]}"; do
    case "$compact_current" in
      *"$expected_source"*) ;;
      *) up_to_date=0; break ;;
    esac
  done
  if [[ "$up_to_date" -eq 1 ]]; then
    echo "  on-chain sources already at $TAG — skipping"
    return 0
  fi

  if [[ "$BROADCAST" != "true" ]]; then
    echo "  DRY-RUN would send:"
    echo "    ETH_PRIVATE_KEY=<owner> cast send $TANGLE_CORE '$SET_SIG' $id '$sources' --rpc-url $RPC_URL"
    return 0
  fi

  # setBlueprintSources is owner-gated; fail fast with a readable error.
  local owner
  owner="$(cast call "$TANGLE_CORE" 'getBlueprint(uint64)((address,address,uint64,uint32,uint8,uint8,bool))' "$id" --rpc-url "$RPC_URL" | sed -E 's/^\((0x[0-9a-fA-F]{40}).*/\1/')"
  if [[ "${owner,,}" != "${SENDER,,}" ]]; then
    echo "ERROR: blueprint $id owner is $owner, sender is $SENDER" >&2
    return 1
  fi

  # Capture output first so a cast failure (revert, RPC error, nonce race)
  # cannot be masked by the cosmetic grep; then require an explicit success
  # status in the receipt. A failed publish must fail the release.
  local send_out
  # cast >=1.7 does not read the signing key from ETH_PRIVATE_KEY for `cast send`;
  # pass it explicitly. Kept out of argv otherwise (value comes from the env var).
  if ! send_out="$(cast send "$TANGLE_CORE" "$SET_SIG" "$id" "$sources" \
    --private-key "$ETH_PRIVATE_KEY" --rpc-url "$RPC_URL")"; then
    echo "ERROR: setBlueprintSources tx failed for blueprint $id" >&2
    return 1
  fi
  grep -E 'transactionHash|status|gasUsed' <<<"$send_out" || true
  if ! grep -qE 'status[[:space:]]+1' <<<"$send_out"; then
    echo "ERROR: setBlueprintSources reverted on-chain for blueprint $id" >&2
    return 1
  fi
  echo "  published $bin@$TAG -> blueprint $id"
}

# Fail fast: a failed publish means the chain/nonce state is unknown —
# continuing could land later blueprints against a stale nonce.
while IFS=: read -r bin id _; do
  [[ -n "$bin" ]] || continue
  [[ -n "$ONLY" && "$ONLY" != "$bin" ]] && continue
  publish_one "$bin" "$id" \
    || { echo "ERROR: aborting after blueprint $bin failure (remaining blueprints not attempted)" >&2; exit 1; }
  echo
done <<< "$CONFIG_ENTRIES"

exit 0
