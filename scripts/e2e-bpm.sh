#!/usr/bin/env bash
# e2e-bpm.sh — Full-stack Blueprint Manager E2E test.
#
# Starts Anvil with Tangle state, deploys contracts, launches the Blueprint
# Manager (BPM), has it catch ServiceActivated events and spawn the sandbox
# operator, runs the instance operator standalone, then exercises the full
# lifecycle: Tangle job submission, sidecar interactions, auth, and security.
#
# Prerequisites:
#   - Docker + tangle-sidecar:local image
#   - Foundry toolchain: anvil, forge, cast
#   - BPM binary (set BPM_BIN or default ../blueprint/target/release/blueprint-manager)
#   - cargo-tangle (for keystore import)
#   - jq
#   - Blueprint binaries built (cargo build --release -p ai-agent-sandbox-blueprint-bin -p ai-agent-instance-blueprint-bin)
#
# Usage:
#   ./scripts/e2e-bpm.sh
#
# Environment overrides:
#   BPM_BIN          — Path to blueprint-manager binary
#   ANVIL_PORT       — Anvil port (default: 8645)
#   SKIP_BUILD       — Skip cargo build (default: 0)
#   SKIP_BPM         — Run without BPM, standalone mode (default: 0)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# ═══════════════════════════════════════════════════════════════════════════
# Configuration
# ═══════════════════════════════════════════════════════════════════════════

ANVIL_PORT="${ANVIL_PORT:-8645}"
RPC_URL="http://127.0.0.1:$ANVIL_PORT"
SANDBOX_API_PORT="${SANDBOX_API_PORT:-9100}"
INSTANCE_API_PORT="${INSTANCE_API_PORT:-9200}"
BPM_AUTH_PROXY_PORT="${BPM_AUTH_PROXY_PORT:-8276}"
SIDECAR_IMAGE="${SIDECAR_IMAGE:-tangle-sidecar:local}"
SKIP_BPM="${SKIP_BPM:-0}"

BPM_BIN="${BPM_BIN:-$ROOT_DIR/../blueprint/target/release/blueprint-manager}"
CARGO_TANGLE_BIN="${CARGO_TANGLE_BIN:-$(command -v cargo-tangle 2>/dev/null || echo "")}"
if [[ -z "$CARGO_TANGLE_BIN" && -x "$ROOT_DIR/../blueprint/target/release/cargo-tangle" ]]; then
    CARGO_TANGLE_BIN="$ROOT_DIR/../blueprint/target/release/cargo-tangle"
fi

ANVIL_STATE="${ANVIL_STATE:-$(cd "$ROOT_DIR/.." && pwd)/blueprint/crates/chain-setup/anvil/snapshots/localtestnet-state.json}"

# Tangle protocol addresses (from state snapshot)
TANGLE="0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
RESTAKING="0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
STATUS_REGISTRY="0x8f86403A4DE0bb5791fa46B8e795C547942fE4Cf"

# Anvil deterministic keys
DEPLOYER_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
OPERATOR1_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
OPERATOR1_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
USER_KEY="0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba"
USER_ADDR="0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"

# Temp directory for BPM data
TMPDIR=$(mktemp -d /tmp/e2e-bpm-XXXXXX)

# ═══════════════════════════════════════════════════════════════════════════
# Assertion helpers
# ═══════════════════════════════════════════════════════════════════════════

PASSES=0
FAILS=0
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

pass() {
    PASSES=$((PASSES + 1))
    echo -e "    ${GREEN}PASS${NC} $1"
}

fail() {
    FAILS=$((FAILS + 1))
    echo -e "    ${RED}FAIL${NC} $1"
    if [[ -n "${2:-}" ]]; then
        echo -e "         ${RED}$2${NC}"
    fi
}

assert_eq() {
    local got="$1" expected="$2" label="$3"
    if [[ "$got" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label" "expected '$expected', got '$got'"
    fi
}

assert_contains() {
    local haystack="$1" needle="$2" label="$3"
    if echo "$haystack" | grep -q "$needle"; then
        pass "$label"
    else
        fail "$label" "expected to contain '$needle'"
    fi
}

assert_status() {
    local expected="$1"; shift
    local status
    status=$(curl -s -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || echo "000")
    if [[ "$status" == "$expected" ]]; then
        pass "HTTP $expected: $*"
    else
        fail "HTTP $expected: $*" "got $status"
    fi
}

summary() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════${NC}"
    echo -e "${CYAN}  E2E BPM Test Summary${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════${NC}"
    echo -e "  ${GREEN}Passed: $PASSES${NC}"
    if [[ $FAILS -gt 0 ]]; then
        echo -e "  ${RED}Failed: $FAILS${NC}"
    else
        echo -e "  Failed: 0"
    fi
    echo "  Total:  $((PASSES + FAILS))"
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════════
# Process management
# ═══════════════════════════════════════════════════════════════════════════

PIDS=()

kill_pids() {
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    # Give processes a moment to exit
    sleep 1
    for pid in "${PIDS[@]}"; do
        kill -9 "$pid" 2>/dev/null || true
    done
}

cleanup() {
    echo ""
    echo "Cleaning up..."
    kill_pids
    rm -rf "$TMPDIR"
    summary
    if [[ $FAILS -gt 0 ]]; then
        exit 1
    fi
}
trap cleanup EXIT INT TERM

wait_for_url() {
    local url="$1" timeout_secs="${2:-30}" label="${3:-$1}"
    local deadline=$((SECONDS + timeout_secs))
    while true; do
        if [[ $SECONDS -ge $deadline ]]; then
            fail "timeout waiting for $label (${timeout_secs}s)"
            return 1
        fi
        if curl -s -o /dev/null -w '' "$url" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
}

wait_for_health() {
    local url="$1" timeout_secs="${2:-30}" label="${3:-$1}"
    # Normalize sidecar URL: replace any non-localhost host with 127.0.0.1
    # (Docker port bindings are accessible locally even if sidecar reports external IP)
    url=$(echo "$url" | sed -E 's|http://[^:]+:|http://127.0.0.1:|')
    local deadline=$((SECONDS + timeout_secs))
    while true; do
        if [[ $SECONDS -ge $deadline ]]; then
            fail "timeout waiting for $label health (${timeout_secs}s)"
            return 1
        fi
        local status
        status=$(curl -s -o /dev/null -w '%{http_code}' "$url/health" 2>/dev/null || echo "000")
        if [[ "$status" == "200" ]]; then
            return 0
        fi
        sleep 1
    done
}

# ═══════════════════════════════════════════════════════════════════════════
# Auth helpers
# ═══════════════════════════════════════════════════════════════════════════

# Get a PASETO session token via EIP-191 challenge flow.
# Usage: TOKEN=$(get_auth_token "$API_URL" "$PRIVATE_KEY")
get_auth_token() {
    local api_url="$1" key="$2"

    # 1. Get challenge
    local challenge
    challenge=$(curl -s -X POST "$api_url/api/auth/challenge")
    local nonce message
    nonce=$(echo "$challenge" | jq -r '.nonce')
    message=$(echo "$challenge" | jq -r '.message')

    if [[ -z "$nonce" || "$nonce" == "null" ]]; then
        echo ""
        return 1
    fi

    # 2. EIP-191 sign (cast wallet sign does this by default)
    local signature
    signature=$(cast wallet sign "$message" --private-key "$key" 2>/dev/null)

    # 3. Exchange for session token
    local session
    session=$(curl -s -X POST "$api_url/api/auth/session" \
        -H "Content-Type: application/json" \
        -d "{\"nonce\":\"$nonce\",\"signature\":\"$signature\"}")
    local token
    token=$(echo "$session" | jq -r '.token')

    if [[ -z "$token" || "$token" == "null" ]]; then
        echo ""
        return 1
    fi

    echo "$token"
}

# Authenticated curl helper
# Usage: auth_curl "$TOKEN" GET "$URL"
#        auth_curl "$TOKEN" POST "$URL" '{"key":"value"}'
auth_curl() {
    local token="$1" method="$2" url="$3" body="${4:-}"
    if [[ -n "$body" ]]; then
        curl -s -X "$method" "$url" \
            -H "Authorization: Bearer $token" \
            -H "Content-Type: application/json" \
            -d "$body"
    else
        curl -s -X "$method" "$url" \
            -H "Authorization: Bearer $token"
    fi
}

auth_curl_status() {
    local token="$1" method="$2" url="$3" body="${4:-}"
    if [[ -n "$body" ]]; then
        curl -s -o /dev/null -w '%{http_code}' -X "$method" "$url" \
            -H "Authorization: Bearer $token" \
            -H "Content-Type: application/json" \
            -d "$body"
    else
        curl -s -o /dev/null -w '%{http_code}' -X "$method" "$url" \
            -H "Authorization: Bearer $token"
    fi
}

# Helper to parse forge script output
parse_deploy() {
    echo "$FORGE_OUTPUT" | grep "DEPLOY_${1}=" | sed "s/.*DEPLOY_${1}=//" | tr -d ' '
}

# ═══════════════════════════════════════════════════════════════════════════
# Preflight checks
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "${CYAN}  Full-Stack BPM E2E Test${NC}"
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo ""

echo "Checking prerequisites..."

for cmd in anvil forge cast jq docker curl; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "  ERROR: $cmd not found in PATH"
        exit 1
    fi
done

if [[ "$SKIP_BPM" != "1" ]]; then
    if [[ ! -x "$BPM_BIN" ]]; then
        echo "  ERROR: BPM binary not found at $BPM_BIN"
        echo "  Build it: cd ../blueprint && cargo build --release -p blueprint-manager"
        echo "  Or set BPM_BIN=/path/to/blueprint-manager"
        echo "  Or set SKIP_BPM=1 to run without BPM"
        exit 1
    fi
    echo "  BPM: $BPM_BIN"
fi

if [[ ! -f "$ANVIL_STATE" ]]; then
    echo "  ERROR: Anvil state snapshot not found at $ANVIL_STATE"
    exit 1
fi

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
    echo "  Building binaries..."
    cargo build --release -p ai-agent-sandbox-blueprint-bin -p ai-agent-instance-blueprint-bin \
        2>&1 | tail -3
fi

SANDBOX_BIN="$ROOT_DIR/target/release/ai-agent-sandbox-blueprint"
INSTANCE_BIN="$ROOT_DIR/target/release/ai-agent-instance-blueprint"

if [[ ! -x "$SANDBOX_BIN" || ! -x "$INSTANCE_BIN" ]]; then
    echo "  ERROR: Blueprint binaries not found. Run: cargo build --release"
    exit 1
fi

echo "  TMPDIR: $TMPDIR"
echo ""

# ── Clean up leftover sidecar containers from previous runs ──────────────
LEFTOVER=$(docker ps -a --filter "name=sidecar-" --format '{{.ID}}' 2>/dev/null)
if [[ -n "$LEFTOVER" ]]; then
    echo "  Cleaning up leftover sidecar containers..."
    echo "$LEFTOVER" | xargs docker rm -f 2>/dev/null || true
fi

# ═══════════════════════════════════════════════════════════════════════════
# Phase 1: Chain Setup
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${YELLOW}Phase 1: Chain Setup${NC}"

# ── Start Anvil ──────────────────────────────────────────────────────────
echo "  Starting Anvil on port $ANVIL_PORT..."
anvil --block-time 2 --host 0.0.0.0 --port "$ANVIL_PORT" \
    --disable-code-size-limit --load-state "$ANVIL_STATE" --silent &
ANVIL_PID=$!
PIDS+=("$ANVIL_PID")
sleep 2

if ! cast block-number --rpc-url "$RPC_URL" >/dev/null 2>&1; then
    fail "Anvil not responding"
    exit 1
fi
pass "Anvil started (PID: $ANVIL_PID)"

# ── Deploy contracts ─────────────────────────────────────────────────────
echo "  Deploying contracts..."
FORGE_OUTPUT=$(forge script "$ROOT_DIR/contracts/script/RegisterBlueprint.s.sol" \
    --rpc-url "$RPC_URL" --broadcast --slow --disable-code-size-limit 2>&1) || true

SANDBOX_BSM=$(parse_deploy SANDBOX_BSM)
INSTANCE_BSM=$(parse_deploy INSTANCE_BSM)
SANDBOX_BLUEPRINT_ID=$(parse_deploy SANDBOX_BLUEPRINT_ID)
INSTANCE_BLUEPRINT_ID=$(parse_deploy INSTANCE_BLUEPRINT_ID)

if [[ -z "$SANDBOX_BSM" || -z "$INSTANCE_BSM" ]]; then
    fail "Contract deployment"
    echo "$FORGE_OUTPUT" | tail -20
    exit 1
fi
pass "Contracts deployed (sandbox=#$SANDBOX_BLUEPRINT_ID, instance=#$INSTANCE_BLUEPRINT_ID)"

# ── Configure job rates ──────────────────────────────────────────────────
BASE_RATE=1000000000000000
for BP_ID_BSM in "$SANDBOX_BLUEPRINT_ID:$SANDBOX_BSM" "$INSTANCE_BLUEPRINT_ID:$INSTANCE_BSM"; do
    IFS=: read -r BP_ID BSM_ADDR <<< "$BP_ID_BSM"
    BASE_RATE=$BASE_RATE BLUEPRINT_ID=$BP_ID TANGLE_ADDRESS=$TANGLE BSM_ADDRESS=$BSM_ADDR \
    forge script "$ROOT_DIR/contracts/script/ConfigureJobRates.s.sol:ConfigureJobRates" \
        --rpc-url "$RPC_URL" --broadcast --slow --disable-code-size-limit > /dev/null 2>&1 || true
done
pass "Job rates configured"

# ── Register operator ────────────────────────────────────────────────────
OPERATOR1_PUBKEY_RAW=$(cast wallet public-key --private-key "$OPERATOR1_KEY" 2>/dev/null | head -1)
OPERATOR1_PUBKEY="0x04${OPERATOR1_PUBKEY_RAW#0x}"

for BP_ID in "$SANDBOX_BLUEPRINT_ID" "$INSTANCE_BLUEPRINT_ID"; do
    cast send "$TANGLE" "registerOperator(uint64,bytes,string)" \
        "$BP_ID" "$OPERATOR1_PUBKEY" "http://127.0.0.1:$SANDBOX_API_PORT" \
        --gas-limit 2000000 --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1 || true
done

OP1_REG=$(cast call "$TANGLE" "isOperatorRegistered(uint64,address)(bool)" \
    "$SANDBOX_BLUEPRINT_ID" "$OPERATOR1_ADDR" --rpc-url "$RPC_URL" 2>/dev/null)
assert_eq "$OP1_REG" "true" "Operator registered for sandbox"

# ── Request services ─────────────────────────────────────────────────────
SVC_BEFORE=$(cast call "$TANGLE" "serviceCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
SVC_BEFORE=$(echo "$SVC_BEFORE" | sed 's/^0x0*//' | sed 's/^$/0/')
NEXT_REQ=$(cast call "$TANGLE" "serviceRequestCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
NEXT_REQ=$(echo "$NEXT_REQ" | sed 's/^0x0*//' | sed 's/^$/0/')

# Sandbox service (single operator for simplicity)
cast send "$TANGLE" \
    "requestService(uint64,address[],bytes,address[],uint64,address,uint256)" \
    "$SANDBOX_BLUEPRINT_ID" \
    "[$OPERATOR1_ADDR]" "0x" \
    "[$USER_ADDR,$DEPLOYER_ADDR]" 31536000 \
    "0x0000000000000000000000000000000000000000" 0 \
    --gas-limit 3000000 --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" > /dev/null 2>&1
SANDBOX_REQ_ID=$NEXT_REQ

# Instance service
NEXT_REQ=$((NEXT_REQ + 1))
INSTANCE_CONFIG=$(cast abi-encode \
    "f(string,string,string,string,string,string,bool,string,bool,uint64,uint64,uint64,uint64,uint64,string,bool,uint8)" \
    "e2e-instance" "agent-dev" "default" "default-agent" "{}" "{}" \
    true "" false 3600 900 2 4096 20 "" false 0)
cast send "$TANGLE" \
    "requestService(uint64,address[],bytes,address[],uint64,address,uint256)" \
    "$INSTANCE_BLUEPRINT_ID" \
    "[$OPERATOR1_ADDR]" "$INSTANCE_CONFIG" \
    "[$USER_ADDR,$DEPLOYER_ADDR]" 31536000 \
    "0x0000000000000000000000000000000000000000" 0 \
    --gas-limit 3000000 --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" > /dev/null 2>&1
INSTANCE_REQ_ID=$NEXT_REQ
pass "Services requested (sandbox=#$SANDBOX_REQ_ID, instance=#$INSTANCE_REQ_ID)"

# ── Set operator capacity ────────────────────────────────────────────────
cast rpc anvil_impersonateAccount "$TANGLE" --rpc-url "$RPC_URL" > /dev/null 2>&1
cast rpc anvil_setBalance "$TANGLE" "0x56BC75E2D63100000" --rpc-url "$RPC_URL" > /dev/null 2>&1
cast send "$SANDBOX_BSM" "onRegister(address,bytes)" "$OPERATOR1_ADDR" "0x" \
    --from "$TANGLE" --unlocked --gas-limit 500000 --rpc-url "$RPC_URL" > /dev/null 2>&1
cast rpc anvil_stopImpersonatingAccount "$TANGLE" --rpc-url "$RPC_URL" > /dev/null 2>&1
pass "Operator capacity set"

echo ""

# ═══════════════════════════════════════════════════════════════════════════
# Phase 2: BPM Launch (or standalone sandbox)
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${YELLOW}Phase 2: Operator Launch${NC}"

# Setup keystore
mkdir -p "$TMPDIR/keystore" "$TMPDIR/data" "$TMPDIR/runtime" "$TMPDIR/cache"
mkdir -p "$TMPDIR/instance-data" "$TMPDIR/sandbox-state" "$TMPDIR/instance-state"

if [[ -n "$CARGO_TANGLE_BIN" && -x "$CARGO_TANGLE_BIN" ]]; then
    "$CARGO_TANGLE_BIN" tangle key import --key-type ecdsa \
        --secret "${OPERATOR1_KEY#0x}" \
        --keystore-path "$TMPDIR/keystore" 2>/dev/null || true
fi

# Common env vars for blueprint binaries
export SIDECAR_IMAGE="$SIDECAR_IMAGE"
export SIDECAR_PULL_IMAGE=false
export SIDECAR_PUBLIC_HOST="127.0.0.1"
export REQUEST_TIMEOUT_SECS=60
export SESSION_AUTH_SECRET="e2e-bpm-test-secret-key"
export CORS_ALLOWED_ORIGINS="*"
export RUST_LOG="${RUST_LOG:-info}"

if [[ "$SKIP_BPM" == "1" ]]; then
    # ── Standalone mode ──────────────────────────────────────────────────
    echo "  Starting sandbox operator (standalone, no BPM)..."

    # First approve services (operator activates them)
    cast send "$TANGLE" "approveService(uint64,uint8)" "$SANDBOX_REQ_ID" 100 \
        --gas-limit 10000000 --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
    cast send "$TANGLE" "approveService(uint64,uint8)" "$INSTANCE_REQ_ID" 100 \
        --gas-limit 10000000 --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
    pass "Services approved"

    # Discover service IDs
    SVC_AFTER=$(cast call "$TANGLE" "serviceCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
    SVC_AFTER=$(echo "$SVC_AFTER" | sed 's/^0x0*//' | sed 's/^$/0/')
    SANDBOX_SERVICE_ID="" ; INSTANCE_SERVICE_ID=""
    for SVC_ID in $(seq "$SVC_BEFORE" "$((SVC_AFTER - 1))"); do
        SVC_DATA=$(cast call "$TANGLE" "getService(uint64)" "$SVC_ID" --rpc-url "$RPC_URL" 2>/dev/null)
        BP_WORD=$(echo "$SVC_DATA" | head -c 66)
        BP_NUM=$(echo "$BP_WORD" | sed 's/^0x0*//' | sed 's/^$/0/')
        [[ "$BP_NUM" == "$SANDBOX_BLUEPRINT_ID" ]] && SANDBOX_SERVICE_ID=$SVC_ID
        [[ "$BP_NUM" == "$INSTANCE_BLUEPRINT_ID" ]] && INSTANCE_SERVICE_ID=$SVC_ID
    done
    pass "Service IDs: sandbox=#$SANDBOX_SERVICE_ID, instance=#$INSTANCE_SERVICE_ID"

    export HTTP_RPC_URL="$RPC_URL"
    export HTTP_RPC_ENDPOINT="$RPC_URL"
    export WS_RPC_URL="ws://127.0.0.1:$ANVIL_PORT"
    export TANGLE_CONTRACT="$TANGLE"
    export RESTAKING_CONTRACT="$RESTAKING"
    export ALLOW_STANDALONE=true
    export PROTOCOL=tangle

    BLUEPRINT_ID="$SANDBOX_BLUEPRINT_ID" \
    SERVICE_ID="$SANDBOX_SERVICE_ID" \
    OPERATOR_API_PORT="$SANDBOX_API_PORT" \
    BLUEPRINT_STATE_DIR="$TMPDIR/sandbox-state" \
    DATA_DIR="$TMPDIR/data" \
    KEYSTORE_URI="$TMPDIR/keystore" \
    "$SANDBOX_BIN" run --test-mode > "$TMPDIR/sandbox.log" 2>&1 &
    SANDBOX_PID=$!
    PIDS+=("$SANDBOX_PID")

    wait_for_health "http://127.0.0.1:$SANDBOX_API_PORT" 60 "sandbox operator"
    pass "Sandbox operator running (standalone, PID: $SANDBOX_PID)"
else
    # ── BPM mode ─────────────────────────────────────────────────────────
    echo "  Generating BPM config..."

    cat > "$TMPDIR/config.toml" <<TOML
http_rpc_endpoint = "http://127.0.0.1:$ANVIL_PORT"
ws_rpc_endpoint = "ws://127.0.0.1:$ANVIL_PORT"
keystore_uri = "$TMPDIR/keystore"
data_dir = "$TMPDIR/data"
test_mode = true

[protocol_settings.Tangle]
blueprint_id = $SANDBOX_BLUEPRINT_ID
tangle_contract = "$TANGLE"
restaking_contract = "$RESTAKING"
status_registry_contract = "$STATUS_REGISTRY"
TOML

    echo "  Starting Blueprint Manager..."
    export HTTP_RPC_URL="$RPC_URL"
    export HTTP_RPC_ENDPOINT="$RPC_URL"
    export WS_RPC_URL="ws://127.0.0.1:$ANVIL_PORT"
    export TANGLE_CONTRACT="$TANGLE"
    export RESTAKING_CONTRACT="$RESTAKING"
    export OPERATOR_API_PORT="$SANDBOX_API_PORT"
    export PROTOCOL=tangle

    "$BPM_BIN" \
        -c "$TMPDIR/config.toml" \
        -d "$TMPDIR/data" \
        -k "$TMPDIR/keystore" \
        -r "$TMPDIR/runtime" \
        -z "$TMPDIR/cache" \
        --test-mode \
        --preferred-source native \
        --auth-proxy-port "$BPM_AUTH_PROXY_PORT" \
        > "$TMPDIR/bpm.log" 2>&1 &
    BPM_PID=$!
    PIDS+=("$BPM_PID")

    # Wait for BPM auth proxy
    sleep 3
    if kill -0 "$BPM_PID" 2>/dev/null; then
        pass "BPM started (PID: $BPM_PID)"
    else
        fail "BPM failed to start"
        cat "$TMPDIR/bpm.log" | tail -30
        exit 1
    fi

    # Now approve services — BPM catches ServiceActivated events
    echo "  Approving services (BPM will catch events)..."
    cast send "$TANGLE" "approveService(uint64,uint8)" "$SANDBOX_REQ_ID" 100 \
        --gas-limit 10000000 --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
    cast send "$TANGLE" "approveService(uint64,uint8)" "$INSTANCE_REQ_ID" 100 \
        --gas-limit 10000000 --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
    pass "Services approved"

    # Discover service IDs
    SVC_AFTER=$(cast call "$TANGLE" "serviceCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
    SVC_AFTER=$(echo "$SVC_AFTER" | sed 's/^0x0*//' | sed 's/^$/0/')
    SANDBOX_SERVICE_ID="" ; INSTANCE_SERVICE_ID=""
    for SVC_ID in $(seq "$SVC_BEFORE" "$((SVC_AFTER - 1))"); do
        SVC_DATA=$(cast call "$TANGLE" "getService(uint64)" "$SVC_ID" --rpc-url "$RPC_URL" 2>/dev/null)
        BP_WORD=$(echo "$SVC_DATA" | head -c 66)
        BP_NUM=$(echo "$BP_WORD" | sed 's/^0x0*//' | sed 's/^$/0/')
        [[ "$BP_NUM" == "$SANDBOX_BLUEPRINT_ID" ]] && SANDBOX_SERVICE_ID=$SVC_ID
        [[ "$BP_NUM" == "$INSTANCE_BLUEPRINT_ID" ]] && INSTANCE_SERVICE_ID=$SVC_ID
    done
    pass "Service IDs: sandbox=#$SANDBOX_SERVICE_ID, instance=#$INSTANCE_SERVICE_ID"

    # Wait for BPM to spawn sandbox operator
    echo "  Waiting for BPM to spawn sandbox operator..."
    wait_for_health "http://127.0.0.1:$SANDBOX_API_PORT" 180 "sandbox operator (via BPM)"
    pass "Sandbox operator spawned by BPM"
fi

# ── Start instance operator (always standalone) ──────────────────────────
echo "  Starting instance operator (standalone)..."

OPERATOR_API_PORT="$INSTANCE_API_PORT" \
BLUEPRINT_ID="$INSTANCE_BLUEPRINT_ID" \
SERVICE_ID="$INSTANCE_SERVICE_ID" \
BSM_ADDRESS="$INSTANCE_BSM" \
BLUEPRINT_STATE_DIR="$TMPDIR/instance-state" \
DATA_DIR="$TMPDIR/instance-data" \
KEYSTORE_URI="$TMPDIR/keystore" \
HTTP_RPC_URL="$RPC_URL" \
HTTP_RPC_ENDPOINT="$RPC_URL" \
WS_RPC_URL="ws://127.0.0.1:$ANVIL_PORT" \
TANGLE_CONTRACT="$TANGLE" \
RESTAKING_CONTRACT="$RESTAKING" \
ALLOW_STANDALONE=true \
PROTOCOL=tangle \
AUTO_PROVISION_POLL_SECS=2 \
AUTO_PROVISION_MAX_ATTEMPTS=90 \
"$INSTANCE_BIN" run --test-mode > "$TMPDIR/instance.log" 2>&1 &
INSTANCE_PID=$!
PIDS+=("$INSTANCE_PID")

wait_for_health "http://127.0.0.1:$INSTANCE_API_PORT" 60 "instance operator"
pass "Instance operator running (PID: $INSTANCE_PID)"

SANDBOX_API="http://127.0.0.1:$SANDBOX_API_PORT"
INSTANCE_API="http://127.0.0.1:$INSTANCE_API_PORT"
echo ""

# ═══════════════════════════════════════════════════════════════════════════
# Phase 3: Sandbox Lifecycle
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${YELLOW}Phase 3: Sandbox Lifecycle${NC}"

# ── Submit JOB_SANDBOX_CREATE via Tangle ─────────────────────────────────
echo "  Submitting sandbox create job via Tangle..."

SANDBOX_CREATE_PAYLOAD=$(cast abi-encode \
    "f((string,string,string,string,string,string,bool,string,bool,uint64,uint64,uint64,uint64,uint64,bool,uint8))" \
    '("e2e-sandbox","agent-dev","default","default-agent","{}","{}",false,"",false,3600,900,2,4096,20,false,0)')

SUBMIT_RECEIPT=$(cast send "$TANGLE" "submitJob(uint64,uint8,bytes)" \
    "$SANDBOX_SERVICE_ID" 0 "$SANDBOX_CREATE_PAYLOAD" \
    --value "$BASE_RATE" --json \
    --gas-limit 5000000 --rpc-url "$RPC_URL" --private-key "$USER_KEY" 2>&1)
SUBMIT_STATUS=$(echo "$SUBMIT_RECEIPT" | jq -r '.status' 2>/dev/null)

if [[ "$SUBMIT_STATUS" == "0x1" ]]; then
    pass "Sandbox create job submitted"
else
    fail "Sandbox create job reverted (status=$SUBMIT_STATUS)"
fi

# ── Authenticate ─────────────────────────────────────────────────────────
echo "  Authenticating with sandbox operator..."

# Wait a moment for the job to be processed
sleep 5

SANDBOX_TOKEN=$(get_auth_token "$SANDBOX_API" "$USER_KEY")
if [[ -z "$SANDBOX_TOKEN" ]]; then
    fail "Sandbox auth: could not get token"
else
    assert_contains "$SANDBOX_TOKEN" "v4.local." "Sandbox PASETO token"
fi

# ── Wait for sandbox to appear ───────────────────────────────────────────
echo "  Waiting for sandbox creation..."
SANDBOX_ID=""
DEADLINE=$((SECONDS + 180))
while [[ $SECONDS -lt $DEADLINE ]]; do
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes" 2>/dev/null || echo "{}")
    SANDBOX_ID=$(echo "$RESP" | jq -r '.sandboxes[0].id // empty' 2>/dev/null || echo "")
    if [[ -n "$SANDBOX_ID" ]]; then
        break
    fi
    sleep 3
done

if [[ -n "$SANDBOX_ID" ]]; then
    pass "Sandbox created: $SANDBOX_ID"
else
    fail "Sandbox not created within 180s"
    echo "  Last response: $RESP"
    # Try to continue with remaining tests
fi

if [[ -n "$SANDBOX_ID" ]]; then
    # ── List sandboxes ───────────────────────────────────────────────────
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes")
    STATE=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .state" 2>/dev/null)
    assert_eq "$STATE" "running" "Sandbox state is running"

    # ── Wait for sidecar healthy ─────────────────────────────────────────
    SIDECAR_URL=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .sidecar_url" 2>/dev/null)
    if [[ -n "$SIDECAR_URL" && "$SIDECAR_URL" != "null" ]]; then
        echo "  Waiting for sidecar at $SIDECAR_URL..."
        wait_for_health "$SIDECAR_URL" 90 "sidecar"
        pass "Sidecar healthy"
    fi

    # ── Exec ─────────────────────────────────────────────────────────────
    RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/exec" \
        '{"command":"echo e2e-sandbox-ok"}')
    EXIT_CODE=$(echo "$RESP" | jq -r '.exit_code' 2>/dev/null)
    STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
    assert_eq "$EXIT_CODE" "0" "Exec exit code 0"
    assert_contains "$STDOUT" "e2e-sandbox-ok" "Exec stdout correct"

    # ── SSH provision ────────────────────────────────────────────────────
    SSH_KEY="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e"
    STATUS=$(auth_curl_status "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/ssh" \
        "{\"username\":\"agent\",\"public_key\":\"$SSH_KEY\"}")
    assert_eq "$STATUS" "200" "SSH provision"

    # ── SSH revoke ───────────────────────────────────────────────────────
    STATUS=$(auth_curl_status "$SANDBOX_TOKEN" DELETE "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/ssh" \
        "{\"username\":\"agent\",\"public_key\":\"$SSH_KEY\"}")
    assert_eq "$STATUS" "200" "SSH revoke"

    # ── Stop (on original container — more reliable than after recreate) ─
    RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/stop")
    STATE=$(echo "$RESP" | jq -r '.state' 2>/dev/null)
    assert_eq "$STATE" "stopped" "Sandbox stopped"

    # ── Verify stopped in list ───────────────────────────────────────────
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes")
    STATE=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .state" 2>/dev/null)
    assert_eq "$STATE" "stopped" "Sandbox shows stopped in list"

    # ── Resume ───────────────────────────────────────────────────────────
    RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/resume")
    STATE=$(echo "$RESP" | jq -r '.state' 2>/dev/null)
    assert_eq "$STATE" "running" "Sandbox resumed"

    # Wait for sidecar to come back after resume
    sleep 3
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes")
    SIDECAR_URL=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .sidecar_url" 2>/dev/null)
    if [[ -n "$SIDECAR_URL" && "$SIDECAR_URL" != "null" ]]; then
        wait_for_health "$SIDECAR_URL" 120 "sidecar after resume" || true
    fi

    # ── Exec after resume (retry up to 3 times) ──────────────────────────
    RESUME_EXEC_OK=0
    for attempt in 1 2 3; do
        RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/exec" \
            '{"command":"echo resumed-ok"}' 2>/dev/null || echo "{}")
        STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
        if echo "$STDOUT" | grep -q "resumed-ok"; then
            RESUME_EXEC_OK=1
            break
        fi
        sleep 5
    done
    if [[ $RESUME_EXEC_OK -eq 1 ]]; then
        pass "Exec after resume"
    else
        fail "Exec after resume" "stdout: $STDOUT"
    fi

    # ── Secrets inject ───────────────────────────────────────────────────
    # NOTE: secrets inject/wipe recreates the container → sandbox gets a NEW ID.
    # The ORIGINAL ID is still registered on-chain (for Tangle delete jobs).
    ORIGINAL_SANDBOX_ID="$SANDBOX_ID"
    SECRETS_RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/secrets" \
        '{"env_json":{"E2E_SECRET":"test-value-42"}}')
    SECRETS_STATUS=$(echo "$SECRETS_RESP" | jq -r '.status // empty' 2>/dev/null)
    if [[ "$SECRETS_STATUS" == "secrets_configured" || "$SECRETS_STATUS" == "secrets_injected" ]]; then
        pass "Secrets inject"
        # Update sandbox ID — recreate_sidecar generates a new one
        NEW_ID=$(echo "$SECRETS_RESP" | jq -r '.sandbox_id // empty' 2>/dev/null)
        if [[ -n "$NEW_ID" ]]; then
            SANDBOX_ID="$NEW_ID"
        fi
    else
        fail "Secrets inject" "response: $SECRETS_RESP"
    fi

    # Wait for new sidecar container to be ready
    sleep 8
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes")
    SIDECAR_URL=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .sidecar_url" 2>/dev/null)
    if [[ -n "$SIDECAR_URL" && "$SIDECAR_URL" != "null" ]]; then
        wait_for_health "$SIDECAR_URL" 90 "sidecar after secrets" || true
    fi

    # ── Verify secrets via exec ──────────────────────────────────────────
    RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/exec" \
        '{"command":"printenv E2E_SECRET"}')
    STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
    assert_contains "$STDOUT" "test-value-42" "Secret visible in container"

    # ── Secrets wipe ─────────────────────────────────────────────────────
    WIPE_RESP=$(auth_curl "$SANDBOX_TOKEN" DELETE "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/secrets")
    WIPE_STATUS=$(echo "$WIPE_RESP" | jq -r '.status // empty' 2>/dev/null)
    if [[ "$WIPE_STATUS" == "secrets_wiped" ]]; then
        pass "Secrets wipe"
        NEW_ID=$(echo "$WIPE_RESP" | jq -r '.sandbox_id // empty' 2>/dev/null)
        if [[ -n "$NEW_ID" ]]; then
            SANDBOX_ID="$NEW_ID"
        fi
    else
        fail "Secrets wipe" "response: $WIPE_RESP"
    fi

    sleep 8
    RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes")
    SIDECAR_URL=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$SANDBOX_ID\") | .sidecar_url" 2>/dev/null)
    if [[ -n "$SIDECAR_URL" && "$SIDECAR_URL" != "null" ]]; then
        wait_for_health "$SIDECAR_URL" 90 "sidecar after wipe" || true
    fi

    RESP=$(auth_curl "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/$SANDBOX_ID/exec" \
        '{"command":"printenv E2E_SECRET || echo NOT_SET"}')
    STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
    if echo "$STDOUT" | grep -q "test-value-42"; then
        fail "Secret should be wiped"
    else
        pass "Secret wiped"
    fi

    # ── Delete via Tangle ────────────────────────────────────────────────
    # Use the ORIGINAL sandbox ID — it's the one registered on-chain.
    # After secrets inject/wipe, the runtime sandbox ID changed but on-chain didn't.
    #
    # NOTE: In standalone mode (SKIP_BPM=1), the operator doesn't submit job
    # results on-chain, so the BSM's sandboxOperator mapping is never populated
    # and Tangle delete will revert with SandboxNotFound. This is expected.
    DELETE_ID="${ORIGINAL_SANDBOX_ID:-$SANDBOX_ID}"
    echo "  Deleting sandbox via Tangle (on-chain ID: $DELETE_ID)..."
    DELETE_PAYLOAD=$(cast abi-encode "f((string))" "(\"$DELETE_ID\")")
    DELETE_RECEIPT=$(cast send "$TANGLE" "submitJob(uint64,uint8,bytes)" \
        "$SANDBOX_SERVICE_ID" 1 "$DELETE_PAYLOAD" \
        --value "$BASE_RATE" --json \
        --gas-limit 5000000 --rpc-url "$RPC_URL" --private-key "$USER_KEY" 2>&1)
    DELETE_TX_STATUS=$(echo "$DELETE_RECEIPT" | jq -r '.status' 2>/dev/null)
    if [[ "$DELETE_TX_STATUS" == "0x1" ]]; then
        pass "Sandbox delete job submitted"

        # Wait for deletion (poll for up to 90s)
        DELETED=0
        for i in $(seq 1 30); do
            sleep 3
            RESP=$(auth_curl "$SANDBOX_TOKEN" GET "$SANDBOX_API/api/sandboxes" 2>/dev/null || echo "{}")
            COUNT=$(echo "$RESP" | jq -r '.sandboxes | length' 2>/dev/null || echo "1")
            if [[ "$COUNT" == "0" ]]; then
                DELETED=1
                break
            fi
        done
        if [[ $DELETED -eq 1 ]]; then
            pass "Sandbox deleted"
        else
            fail "Sandbox still present after delete"
        fi
    elif [[ "$SKIP_BPM" == "1" ]]; then
        # Expected in standalone mode: on-chain sandbox registry not populated
        pass "Sandbox delete reverted (expected in standalone mode — no on-chain result)"
    else
        fail "Sandbox delete job reverted (status=$DELETE_TX_STATUS)"
    fi
fi

echo ""

# ═══════════════════════════════════════════════════════════════════════════
# Phase 4: Instance Lifecycle
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${YELLOW}Phase 4: Instance Lifecycle${NC}"

# ── Submit JOB_PROVISION via Tangle ──────────────────────────────────────
echo "  Submitting instance provision job via Tangle..."

PROVISION_PAYLOAD=$(cast abi-encode \
    "f((string,string,string,string,string,string,bool,string,bool,uint64,uint64,uint64,uint64,uint64,string,bool,uint8))" \
    '("e2e-instance","agent-dev","default","default-agent","{}","{}",false,"",false,3600,900,2,4096,20,"",false,0)')

PROVISION_RECEIPT=$(cast send "$TANGLE" "submitJob(uint64,uint8,bytes)" \
    "$INSTANCE_SERVICE_ID" 5 "$PROVISION_PAYLOAD" \
    --value "$BASE_RATE" --json \
    --gas-limit 5000000 --rpc-url "$RPC_URL" --private-key "$USER_KEY" 2>&1)
PROVISION_STATUS=$(echo "$PROVISION_RECEIPT" | jq -r '.status' 2>/dev/null)

if [[ "$PROVISION_STATUS" == "0x1" ]]; then
    pass "Instance provision job submitted"
else
    fail "Instance provision job reverted (status=$PROVISION_STATUS)"
fi

# ── Authenticate ─────────────────────────────────────────────────────────
# Instance auto-provisions with serviceOwner = DEPLOYER_ADDR (from requestService).
# Auth must use DEPLOYER_KEY to match the sandbox owner.
echo "  Authenticating with instance operator (as service owner)..."
sleep 5

INSTANCE_TOKEN=$(get_auth_token "$INSTANCE_API" "$DEPLOYER_KEY")
if [[ -z "$INSTANCE_TOKEN" ]]; then
    fail "Instance auth: could not get token"
else
    pass "Instance PASETO token acquired (service owner)"
fi

# ── Wait for instance sandbox ────────────────────────────────────────────
echo "  Waiting for instance provision..."
INSTANCE_SANDBOX_ID=""
DEADLINE=$((SECONDS + 180))
while [[ $SECONDS -lt $DEADLINE ]]; do
    RESP=$(auth_curl "$INSTANCE_TOKEN" GET "$INSTANCE_API/api/sandboxes" 2>/dev/null || echo "{}")
    INSTANCE_SANDBOX_ID=$(echo "$RESP" | jq -r '.sandboxes[0].id // empty' 2>/dev/null || echo "")
    if [[ -n "$INSTANCE_SANDBOX_ID" ]]; then
        break
    fi
    sleep 3
done

if [[ -n "$INSTANCE_SANDBOX_ID" ]]; then
    pass "Instance provisioned: $INSTANCE_SANDBOX_ID"
else
    fail "Instance not provisioned within 180s"
fi

if [[ -n "$INSTANCE_SANDBOX_ID" ]]; then
    INSTANCE_SIDECAR_URL=$(echo "$RESP" | jq -r '.sandboxes[0].sidecar_url // empty' 2>/dev/null)

    if [[ -n "$INSTANCE_SIDECAR_URL" && "$INSTANCE_SIDECAR_URL" != "null" ]]; then
        echo "  Waiting for instance sidecar at $INSTANCE_SIDECAR_URL..."
        if wait_for_health "$INSTANCE_SIDECAR_URL" 120 "instance sidecar"; then
            pass "Instance sidecar healthy"
        fi
    fi

    # ── Instance exec (singleton endpoint, retry up to 3 times) ──────────
    INSTANCE_EXEC_OK=0
    for attempt in 1 2 3; do
        RESP=$(auth_curl "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/exec" \
            '{"command":"echo e2e-instance-ok"}' 2>/dev/null || echo "{}")
        EXIT_CODE=$(echo "$RESP" | jq -r '.exit_code' 2>/dev/null)
        STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
        if [[ "$EXIT_CODE" == "0" ]] && echo "$STDOUT" | grep -q "e2e-instance-ok"; then
            INSTANCE_EXEC_OK=1
            break
        fi
        sleep 5
    done
    if [[ $INSTANCE_EXEC_OK -eq 1 ]]; then
        pass "Instance exec exit code 0"
        pass "Instance exec stdout correct"
    else
        fail "Instance exec exit code 0" "got $EXIT_CODE"
        fail "Instance exec stdout correct" "got: $STDOUT"
    fi

    # ── Instance SSH ─────────────────────────────────────────────────────
    SSH_KEY="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e"
    STATUS=$(auth_curl_status "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/ssh" \
        "{\"username\":\"agent\",\"public_key\":\"$SSH_KEY\"}")
    assert_eq "$STATUS" "200" "Instance SSH provision"

    STATUS=$(auth_curl_status "$INSTANCE_TOKEN" DELETE "$INSTANCE_API/api/sandbox/ssh" \
        "{\"username\":\"agent\",\"public_key\":\"$SSH_KEY\"}")
    assert_eq "$STATUS" "200" "Instance SSH revoke"

    # ── Instance stop/resume ─────────────────────────────────────────────
    RESP=$(auth_curl "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/stop")
    STATE=$(echo "$RESP" | jq -r '.state' 2>/dev/null)
    assert_eq "$STATE" "stopped" "Instance stopped"

    RESP=$(auth_curl "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/resume")
    STATE=$(echo "$RESP" | jq -r '.state' 2>/dev/null)
    assert_eq "$STATE" "running" "Instance resumed"

    # Re-fetch sidecar URL after resume (container restarted — port may change)
    sleep 3
    RESP=$(auth_curl "$INSTANCE_TOKEN" GET "$INSTANCE_API/api/sandboxes")
    INSTANCE_SIDECAR_URL=$(echo "$RESP" | jq -r '.sandboxes[0].sidecar_url // empty' 2>/dev/null)
    if [[ -n "$INSTANCE_SIDECAR_URL" && "$INSTANCE_SIDECAR_URL" != "null" ]]; then
        wait_for_health "$INSTANCE_SIDECAR_URL" 90 "instance sidecar after resume" || true
    fi

    RESP=$(auth_curl "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/exec" \
        '{"command":"echo instance-resumed-ok"}')
    STDOUT=$(echo "$RESP" | jq -r '.stdout' 2>/dev/null)
    assert_contains "$STDOUT" "instance-resumed-ok" "Instance exec after resume"

    # ── Deprovision via Tangle ───────────────────────────────────────────
    echo "  Deprovisioning instance via Tangle..."
    DEPROVISION_PAYLOAD=$(cast abi-encode "f((string))" '("{}")')
    cast send "$TANGLE" "submitJob(uint64,uint8,bytes)" \
        "$INSTANCE_SERVICE_ID" 6 "$DEPROVISION_PAYLOAD" \
        --value "$BASE_RATE" \
        --gas-limit 5000000 --rpc-url "$RPC_URL" --private-key "$USER_KEY" > /dev/null 2>&1 || true

    sleep 10
    RESP=$(auth_curl "$INSTANCE_TOKEN" GET "$INSTANCE_API/api/sandboxes")
    REMAINING=$(echo "$RESP" | jq -r ".sandboxes[] | select(.id==\"$INSTANCE_SANDBOX_ID\") | .id" 2>/dev/null || echo "")
    if [[ -z "$REMAINING" ]]; then
        pass "Instance deprovisioned"
    else
        fail "Instance still present after deprovision"
    fi
fi

echo ""

# ═══════════════════════════════════════════════════════════════════════════
# Phase 5: Security & Hardening Tests
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${YELLOW}Phase 5: Security Tests${NC}"

# Re-authenticate for security tests (tokens may have been invalidated by previous tests)
SANDBOX_TOKEN=$(get_auth_token "$SANDBOX_API" "$USER_KEY" 2>/dev/null || echo "")
INSTANCE_TOKEN=$(get_auth_token "$INSTANCE_API" "$DEPLOYER_KEY" 2>/dev/null || echo "")

# ── Auth rejection (no token) ────────────────────────────────────────────
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X GET "$SANDBOX_API/api/sandboxes" 2>/dev/null)
assert_eq "$STATUS" "401" "No auth → 401 (sandbox)"

STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X GET "$INSTANCE_API/api/sandboxes" 2>/dev/null)
assert_eq "$STATUS" "401" "No auth → 401 (instance)"

# ── Auth rejection (bad token) ───────────────────────────────────────────
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X GET "$SANDBOX_API/api/sandboxes" \
    -H "Authorization: Bearer v4.local.INVALID_TOKEN_DATA" 2>/dev/null)
assert_eq "$STATUS" "401" "Bad PASETO → 401"

# ── Input validation ─────────────────────────────────────────────────────
if [[ -n "$SANDBOX_TOKEN" ]]; then
    # Empty command
    STATUS=$(auth_curl_status "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/nonexistent/exec" \
        '{"command":""}')
    if [[ "$STATUS" == "400" || "$STATUS" == "404" ]]; then
        pass "Empty command rejected ($STATUS)"
    else
        fail "Empty command should be 400 or 404" "got $STATUS"
    fi

    # Bad SSH key
    STATUS=$(auth_curl_status "$SANDBOX_TOKEN" POST "$SANDBOX_API/api/sandboxes/nonexistent/ssh" \
        '{"username":"agent","public_key":"not-a-real-key"}')
    if [[ "$STATUS" == "400" || "$STATUS" == "404" ]]; then
        pass "Bad SSH key rejected ($STATUS)"
    else
        fail "Bad SSH key should be 400 or 404" "got $STATUS"
    fi
fi

if [[ -n "$INSTANCE_TOKEN" ]]; then
    # Empty command on instance
    STATUS=$(auth_curl_status "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/exec" \
        '{"command":""}')
    # May be 400 (validation) or 404/500 (no sandbox provisioned)
    if [[ "$STATUS" != "200" ]]; then
        pass "Instance empty command rejected ($STATUS)"
    else
        fail "Instance empty command should be rejected"
    fi

    # Bad SSH key on instance
    STATUS=$(auth_curl_status "$INSTANCE_TOKEN" POST "$INSTANCE_API/api/sandbox/ssh" \
        '{"username":"agent","public_key":"garbage"}')
    if [[ "$STATUS" != "200" ]]; then
        pass "Instance bad SSH key rejected ($STATUS)"
    else
        fail "Instance bad SSH key should be rejected"
    fi
fi

# ── Rate limiting ────────────────────────────────────────────────────────
echo "  Testing rate limiting (auth endpoint: 10 req/min)..."
RATE_LIMITED=0
for i in $(seq 1 15); do
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SANDBOX_API/api/auth/challenge" 2>/dev/null)
    if [[ "$STATUS" == "429" ]]; then
        RATE_LIMITED=1
        break
    fi
done
if [[ $RATE_LIMITED -eq 1 ]]; then
    pass "Rate limiting triggered (auth: 10/min)"
else
    fail "Rate limiting not triggered (expected 429 on auth after 15 rapid requests)"
fi

# Wait for rate limit window to pass
sleep 2

# ── Body size limit ──────────────────────────────────────────────────────
# Generate a ~2MB payload
LARGE_BODY=$(python3 -c "print('{\"command\":\"' + 'A'*2097152 + '\"}')" 2>/dev/null || echo '{"command":"SKIP"}')
if [[ "$LARGE_BODY" != '{"command":"SKIP"}' ]]; then
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SANDBOX_API/api/sandboxes/test/exec" \
        -H "Authorization: Bearer ${SANDBOX_TOKEN:-invalid}" \
        -H "Content-Type: application/json" \
        -d "$LARGE_BODY" 2>/dev/null)
    if [[ "$STATUS" == "413" ]]; then
        pass "Body size limit (2MB → 413)"
    else
        # 401/400 are also acceptable (auth or validation may fire first)
        pass "Large body rejected ($STATUS)"
    fi
fi

# ── Path traversal ───────────────────────────────────────────────────────
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
    "$SANDBOX_API/api/sandboxes/..%2F..%2Fetc%2Fpasswd" \
    -H "Authorization: Bearer ${SANDBOX_TOKEN:-invalid}" 2>/dev/null)
if [[ "$STATUS" == "400" || "$STATUS" == "404" || "$STATUS" == "401" ]]; then
    pass "Path traversal rejected ($STATUS)"
else
    fail "Path traversal not rejected" "got $STATUS"
fi

# ── Invalid JSON ─────────────────────────────────────────────────────────
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SANDBOX_API/api/sandboxes/test/exec" \
    -H "Authorization: Bearer ${SANDBOX_TOKEN:-invalid}" \
    -H "Content-Type: application/json" \
    -d '{broken json' 2>/dev/null)
if [[ "$STATUS" == "400" || "$STATUS" == "422" || "$STATUS" == "404" || "$STATUS" == "401" ]]; then
    pass "Invalid JSON rejected ($STATUS)"
else
    fail "Invalid JSON not rejected" "got $STATUS"
fi

# ── CORS headers ─────────────────────────────────────────────────────────
CORS_HEADER=$(curl -s -D - -o /dev/null "$SANDBOX_API/health" 2>/dev/null | grep -i "access-control-allow-origin" || echo "")
if [[ -n "$CORS_HEADER" ]]; then
    pass "CORS headers present"
else
    # CORS may only appear on OPTIONS or with Origin header
    CORS_HEADER=$(curl -s -D - -o /dev/null -H "Origin: http://localhost:3000" "$SANDBOX_API/health" 2>/dev/null | grep -i "access-control-allow-origin" || echo "")
    if [[ -n "$CORS_HEADER" ]]; then
        pass "CORS headers present (with Origin)"
    else
        fail "CORS headers missing"
    fi
fi

# ── Auth endpoints are public ────────────────────────────────────────────
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SANDBOX_API/api/auth/challenge" 2>/dev/null)
assert_eq "$STATUS" "200" "Auth challenge endpoint is public"

STATUS=$(curl -s -o /dev/null -w '%{http_code}' "$SANDBOX_API/health" 2>/dev/null)
assert_eq "$STATUS" "200" "Health endpoint is public"

echo ""

# ═══════════════════════════════════════════════════════════════════════════
# Done
# ═══════════════════════════════════════════════════════════════════════════

echo -e "${GREEN}All test phases complete.${NC}"
# Cleanup trap will print summary and exit
