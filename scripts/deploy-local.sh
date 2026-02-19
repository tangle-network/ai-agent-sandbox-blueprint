#!/usr/bin/env bash
# deploy-local.sh — Full Tangle blueprint lifecycle deployment on local Anvil.
#
# Deploys all 3 blueprint contracts, registers them on Tangle, sets up 2
# operators, requests + approves services, configures pricing, builds binaries,
# starts the sandbox operator API, and writes a .env.local for the UI.
#
# Prerequisites:
#   - Foundry toolchain: anvil, forge, cast
#   - Tangle Anvil state snapshot at:
#       ../blueprint/crates/chain-setup/anvil/snapshots/localtestnet-state.json
#   - Docker (for sidecar containers)
#   - cargo-tangle (optional, for keystore import)
#
# Usage:
#   ./scripts/deploy-local.sh
#
# Environment overrides:
#   RPC_URL              — Anvil RPC URL (default: http://127.0.0.1:8545)
#   ANVIL_PORT           — Anvil port (default: 8545)
#   OPERATOR_API_PORT    — Operator 1 API port (default: 9090)
#   SIDECAR_IMAGE        — Docker image for sidecars (default: tangle-sidecar:local)
#   SKIP_BUILD           — Set to 1 to skip cargo build
#   BASE_RATE            — Per-job base rate in wei (default: 1e15 = 0.001 TNT)
#   ANVIL_STATE          — Path to Anvil state snapshot
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ANVIL_PORT="${ANVIL_PORT:-8545}"
RPC_URL="${RPC_URL:-http://127.0.0.1:$ANVIL_PORT}"
SIDECAR_IMAGE="${SIDECAR_IMAGE:-tangle-sidecar:local}"
OPERATOR_API_PORT="${OPERATOR_API_PORT:-9090}"
BASE_RATE="${BASE_RATE:-1000000000000000}" # 1e15 = 0.001 TNT

# Anvil state snapshot (Tangle protocol pre-deployed)
ANVIL_STATE="${ANVIL_STATE:-$(cd "$ROOT_DIR/.." && pwd)/blueprint/crates/chain-setup/anvil/snapshots/localtestnet-state.json}"

# Anvil deterministic accounts
DEPLOYER_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
OPERATOR1_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
OPERATOR1_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
OPERATOR2_KEY="0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
OPERATOR2_ADDR="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
USER_KEY="0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba"
USER_ADDR="0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"

# Tangle protocol addresses (from state snapshot)
TANGLE="0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
RESTAKING="0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"

cleanup() {
    echo ""
    echo "Shutting down..."
    [ -n "${ANVIL_PID:-}" ] && kill "$ANVIL_PID" 2>/dev/null || true
    [ -n "${OPERATOR_PID:-}" ] && kill "$OPERATOR_PID" 2>/dev/null || true
    exit 0
}
trap cleanup INT TERM

echo "=== AI Agent Sandbox Blueprint — Full Local Deployment ==="
echo "RPC: $RPC_URL"
echo "Tangle: $TANGLE"
echo ""

# Helper to parse forge script output
parse_deploy() {
    echo "$FORGE_OUTPUT" | grep "DEPLOY_${1}=" | sed "s/.*DEPLOY_${1}=//" | tr -d ' '
}

# ── [0/10] Start Anvil with Tangle state ────────────────────────────
echo "[0/10] Starting Anvil with Tangle protocol state..."
if [ -f "$ANVIL_STATE" ]; then
    anvil --block-time 2 --port "$ANVIL_PORT" --load-state "$ANVIL_STATE" --silent &
    ANVIL_PID=$!
    echo "  Loaded state from: $ANVIL_STATE"
else
    echo "  WARNING: State snapshot not found at $ANVIL_STATE"
    echo "  Starting fresh Anvil (Tangle protocol will NOT be available)"
    anvil --block-time 2 --port "$ANVIL_PORT" --silent &
    ANVIL_PID=$!
fi
sleep 2

if ! cast block-number --rpc-url "$RPC_URL" >/dev/null 2>&1; then
    echo "ERROR: Anvil not responding on $RPC_URL"
    exit 1
fi
echo "  Anvil running (PID: $ANVIL_PID)"

# ── [1/10] Deploy contracts + register blueprints on Tangle ─────────
echo "[1/10] Deploying contracts + registering blueprints on Tangle..."
FORGE_OUTPUT=$(forge script "$ROOT_DIR/contracts/script/RegisterBlueprint.s.sol" \
    --rpc-url "$RPC_URL" --broadcast --slow 2>&1) || {
    echo "ERROR: Deployment failed. Output:"
    echo "$FORGE_OUTPUT" | tail -40
    exit 1
}

SANDBOX_BSM=$(parse_deploy SANDBOX_BSM)
INSTANCE_BSM=$(parse_deploy INSTANCE_BSM)
TEE_INSTANCE_BSM=$(parse_deploy TEE_INSTANCE_BSM)
SANDBOX_BLUEPRINT_ID=$(parse_deploy SANDBOX_BLUEPRINT_ID)
INSTANCE_BLUEPRINT_ID=$(parse_deploy INSTANCE_BLUEPRINT_ID)
TEE_INSTANCE_BLUEPRINT_ID=$(parse_deploy TEE_INSTANCE_BLUEPRINT_ID)

if [[ -z "$SANDBOX_BSM" || -z "$INSTANCE_BSM" || -z "$SANDBOX_BLUEPRINT_ID" ]]; then
    echo "ERROR: Failed to parse deployment output."
    echo "$FORGE_OUTPUT" | tail -30
    exit 1
fi

echo "  Sandbox BSM:       $SANDBOX_BSM (blueprint #$SANDBOX_BLUEPRINT_ID)"
echo "  Instance BSM:      $INSTANCE_BSM (blueprint #$INSTANCE_BLUEPRINT_ID)"
echo "  TEE Instance BSM:  $TEE_INSTANCE_BSM (blueprint #$TEE_INSTANCE_BLUEPRINT_ID)"

# ── [2/10] Configure per-job pricing ────────────────────────────────
echo "[2/10] Configuring per-job pricing (base rate: $BASE_RATE wei)..."

# Sandbox blueprint (EventDriven pricing)
BASE_RATE=$BASE_RATE \
BLUEPRINT_ID=$SANDBOX_BLUEPRINT_ID \
TANGLE_ADDRESS=$TANGLE \
BSM_ADDRESS=$SANDBOX_BSM \
forge script "$ROOT_DIR/contracts/script/ConfigureJobRates.s.sol:ConfigureJobRates" \
    --rpc-url "$RPC_URL" --broadcast --slow > /dev/null 2>&1
echo "  Sandbox: 17 job rates configured"

# Instance blueprint
BASE_RATE=$BASE_RATE \
BLUEPRINT_ID=$INSTANCE_BLUEPRINT_ID \
TANGLE_ADDRESS=$TANGLE \
BSM_ADDRESS=$INSTANCE_BSM \
forge script "$ROOT_DIR/contracts/script/ConfigureInstanceJobRates.s.sol:ConfigureInstanceJobRates" \
    --rpc-url "$RPC_URL" --broadcast --slow > /dev/null 2>&1
echo "  Instance: 8 job rates configured"

# TEE Instance blueprint
BASE_RATE=$BASE_RATE \
BLUEPRINT_ID=$TEE_INSTANCE_BLUEPRINT_ID \
TANGLE_ADDRESS=$TANGLE \
BSM_ADDRESS=$TEE_INSTANCE_BSM \
forge script "$ROOT_DIR/contracts/script/ConfigureTeeInstanceJobRates.s.sol:ConfigureTeeInstanceJobRates" \
    --rpc-url "$RPC_URL" --broadcast --slow > /dev/null 2>&1
echo "  TEE Instance: 8 job rates configured"

# ── [3/10] Register operators ────────────────────────────────────────
echo "[3/10] Registering operators on Tangle..."

# Derive ECDSA public keys from private keys using cast
OPERATOR1_PUBKEY=$(cast wallet address --private-key "$OPERATOR1_KEY" 2>/dev/null | head -1)
OPERATOR2_PUBKEY=$(cast wallet address --private-key "$OPERATOR2_KEY" 2>/dev/null | head -1)

# Register both operators for the Sandbox blueprint
for BLUEPRINT_ID in "$SANDBOX_BLUEPRINT_ID" "$INSTANCE_BLUEPRINT_ID" "$TEE_INSTANCE_BLUEPRINT_ID"; do
    # Operator 1
    cast send "$TANGLE" \
        "registerOperator(uint64,bytes,string)" \
        "$BLUEPRINT_ID" "0x" "http://localhost:$OPERATOR_API_PORT" \
        --gas-limit 500000 \
        --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1 || true

    # Operator 2
    cast send "$TANGLE" \
        "registerOperator(uint64,bytes,string)" \
        "$BLUEPRINT_ID" "0x" "http://localhost:$((OPERATOR_API_PORT + 1))" \
        --gas-limit 500000 \
        --rpc-url "$RPC_URL" --private-key "$OPERATOR2_KEY" > /dev/null 2>&1 || true
done

echo "  Operator 1: $OPERATOR1_ADDR → http://localhost:$OPERATOR_API_PORT"
echo "  Operator 2: $OPERATOR2_ADDR → http://localhost:$((OPERATOR_API_PORT + 1))"

# ── [4/10] Request services ──────────────────────────────────────────
echo "[4/10] Requesting services..."

# Get the next request IDs
NEXT_REQ=$(cast call "$TANGLE" "serviceRequestCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
NEXT_REQ=$(echo "$NEXT_REQ" | sed 's/^0x//' | sed 's/^0*//' | sed 's/^$/0/')

# Request sandbox service (Dynamic membership, EventDriven pricing → no payment)
cast send "$TANGLE" \
    "requestService(uint64,address[],bytes,address[],uint64,address,uint256)" \
    "$SANDBOX_BLUEPRINT_ID" \
    "[$OPERATOR1_ADDR,$OPERATOR2_ADDR]" \
    "0x" \
    "[$USER_ADDR,$DEPLOYER_ADDR]" \
    31536000 \
    "0x0000000000000000000000000000000000000000" \
    0 \
    --gas-limit 3000000 \
    --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" > /dev/null 2>&1
SANDBOX_REQ_ID=$NEXT_REQ
echo "  Sandbox service request #$SANDBOX_REQ_ID submitted"

# Request instance service
NEXT_REQ=$((NEXT_REQ + 1))
cast send "$TANGLE" \
    "requestService(uint64,address[],bytes,address[],uint64,address,uint256)" \
    "$INSTANCE_BLUEPRINT_ID" \
    "[$OPERATOR1_ADDR,$OPERATOR2_ADDR]" \
    "0x" \
    "[$USER_ADDR,$DEPLOYER_ADDR]" \
    31536000 \
    "0x0000000000000000000000000000000000000000" \
    0 \
    --gas-limit 3000000 \
    --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" > /dev/null 2>&1
INSTANCE_REQ_ID=$NEXT_REQ
echo "  Instance service request #$INSTANCE_REQ_ID submitted"

# ── [5/10] Operators approve services ────────────────────────────────
echo "[5/10] Operators approving services..."

# Approve sandbox service
cast send "$TANGLE" "approveService(uint64,uint8)" "$SANDBOX_REQ_ID" 100 \
    --gas-limit 10000000 \
    --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
cast send "$TANGLE" "approveService(uint64,uint8)" "$SANDBOX_REQ_ID" 100 \
    --gas-limit 10000000 \
    --rpc-url "$RPC_URL" --private-key "$OPERATOR2_KEY" > /dev/null 2>&1
echo "  Sandbox service: both operators approved"

# Approve instance service
cast send "$TANGLE" "approveService(uint64,uint8)" "$INSTANCE_REQ_ID" 100 \
    --gas-limit 10000000 \
    --rpc-url "$RPC_URL" --private-key "$OPERATOR1_KEY" > /dev/null 2>&1
cast send "$TANGLE" "approveService(uint64,uint8)" "$INSTANCE_REQ_ID" 100 \
    --gas-limit 10000000 \
    --rpc-url "$RPC_URL" --private-key "$OPERATOR2_KEY" > /dev/null 2>&1
echo "  Instance service: both operators approved"

# Read service IDs
SERVICE_COUNT=$(cast call "$TANGLE" "serviceCount()(uint64)" --rpc-url "$RPC_URL" 2>&1 | xargs)
SERVICE_COUNT=$(echo "$SERVICE_COUNT" | sed 's/^0x//' | sed 's/^0*//' | sed 's/^$/0/')
INSTANCE_SERVICE_ID=$((SERVICE_COUNT - 1))
SANDBOX_SERVICE_ID=$((SERVICE_COUNT - 2))

echo "  Sandbox service ID: $SANDBOX_SERVICE_ID"
echo "  Instance service ID: $INSTANCE_SERVICE_ID"

# ── [6/10] Set operator capacity (Sandbox blueprint) ────────────────
echo "[6/10] Setting operator capacity..."

# The Sandbox BSM needs operator capacity set. onRegister does this when
# inputs are provided, but for Anvil testing we set it directly.
cast rpc anvil_impersonateAccount "$TANGLE" --rpc-url "$RPC_URL" > /dev/null 2>&1
cast rpc anvil_setBalance "$TANGLE" "0x56BC75E2D63100000" --rpc-url "$RPC_URL" > /dev/null 2>&1

# Trigger onRegister for sandbox operators so capacity gets set
cast send "$SANDBOX_BSM" "onRegister(address,bytes)" "$OPERATOR1_ADDR" "0x" \
    --from "$TANGLE" --unlocked --gas-limit 500000 \
    --rpc-url "$RPC_URL" > /dev/null 2>&1
cast send "$SANDBOX_BSM" "onRegister(address,bytes)" "$OPERATOR2_ADDR" "0x" \
    --from "$TANGLE" --unlocked --gas-limit 500000 \
    --rpc-url "$RPC_URL" > /dev/null 2>&1

cast rpc anvil_stopImpersonatingAccount "$TANGLE" --rpc-url "$RPC_URL" > /dev/null 2>&1

OP1_CAP=$(cast call "$SANDBOX_BSM" "operatorMaxCapacity(address)(uint32)" "$OPERATOR1_ADDR" --rpc-url "$RPC_URL" 2>&1 | xargs)
OP2_CAP=$(cast call "$SANDBOX_BSM" "operatorMaxCapacity(address)(uint32)" "$OPERATOR2_ADDR" --rpc-url "$RPC_URL" 2>&1 | xargs)
echo "  Operator 1 capacity: $OP1_CAP"
echo "  Operator 2 capacity: $OP2_CAP"

# ── [7/10] Setup keystores ──────────────────────────────────────────
echo "[7/10] Setting up operator keystores..."

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$SCRIPTS_DIR/data/operator1/keystore" "$SCRIPTS_DIR/data/operator2/keystore"

CARGO_TANGLE="${CARGO_TANGLE_BIN:-$(command -v cargo-tangle 2>/dev/null || echo "")}"
if [[ -z "$CARGO_TANGLE" && -x "$ROOT_DIR/../blueprint/target/release/cargo-tangle" ]]; then
    CARGO_TANGLE="$ROOT_DIR/../blueprint/target/release/cargo-tangle"
fi

if [[ -n "$CARGO_TANGLE" && -x "$CARGO_TANGLE" ]]; then
    "$CARGO_TANGLE" tangle key import --key-type ecdsa \
        --secret "${OPERATOR1_KEY#0x}" \
        --keystore-path "$SCRIPTS_DIR/data/operator1/keystore" 2>/dev/null || true
    "$CARGO_TANGLE" tangle key import --key-type ecdsa \
        --secret "${OPERATOR2_KEY#0x}" \
        --keystore-path "$SCRIPTS_DIR/data/operator2/keystore" 2>/dev/null || true
    echo "  Keys imported via cargo-tangle"
else
    echo "  WARNING: cargo-tangle not found, skipping keystore import"
    echo "  Build it: cd ../blueprint && cargo build -p cargo-tangle --release"
fi

# ── [8/10] Build operator binary ─────────────────────────────────────
if [[ "${SKIP_BUILD:-0}" == "1" ]]; then
    echo "[8/10] Skipping build (SKIP_BUILD=1)"
else
    echo "[8/10] Building sandbox operator binary..."
    cargo build --release -p ai-agent-sandbox-blueprint-bin 2>&1 | tail -3
    echo "  Binary built"
fi

# ── [9/10] Start operator ───────────────────────────────────────────
echo "[9/10] Starting sandbox operator..."

export HTTP_RPC_ENDPOINT="$RPC_URL"
export WS_RPC_ENDPOINT="ws://127.0.0.1:$ANVIL_PORT"
export BLUEPRINT_ID="$SANDBOX_BLUEPRINT_ID"
export SERVICE_ID="$SANDBOX_SERVICE_ID"
export OPERATOR_API_PORT="$OPERATOR_API_PORT"
export SIDECAR_IMAGE="$SIDECAR_IMAGE"
export SIDECAR_PULL_IMAGE=false
export SIDECAR_PUBLIC_HOST="127.0.0.1"
export REQUEST_TIMEOUT_SECS=60
export SESSION_AUTH_SECRET="dev-secret-key-do-not-use-in-production"
export ALLOW_STANDALONE=true
export CORS_ALLOWED_ORIGINS="*"
export RUST_LOG="${RUST_LOG:-info}"
export KEYSTORE_URI="file://$SCRIPTS_DIR/data/operator1/keystore"

"$ROOT_DIR/target/release/ai-agent-sandbox-blueprint-bin" run &
OPERATOR_PID=$!

# Wait for operator API to be ready
DEADLINE=$((SECONDS + 30))
until curl -sf "http://localhost:$OPERATOR_API_PORT/api/sandboxes" > /dev/null 2>&1; do
    if [ $SECONDS -ge $DEADLINE ]; then
        echo "  WARNING: Operator API not ready within 30s (may still be starting)"
        break
    fi
    sleep 1
done
echo "  Operator running (PID: $OPERATOR_PID)"

# ── [10/10] Write env file ──────────────────────────────────────────
echo "[10/10] Writing .env.local..."

cat > "$ROOT_DIR/.env.local" <<EOF
# Generated by deploy-local.sh — do not edit
# RPC
HTTP_RPC_ENDPOINT=$RPC_URL
WS_RPC_ENDPOINT=ws://127.0.0.1:$ANVIL_PORT

# Tangle protocol
TANGLE_CONTRACT=$TANGLE
RESTAKING_CONTRACT=$RESTAKING

# Blueprint addresses
SANDBOX_BSM=$SANDBOX_BSM
INSTANCE_BSM=$INSTANCE_BSM
TEE_INSTANCE_BSM=$TEE_INSTANCE_BSM

# Blueprint IDs
SANDBOX_BLUEPRINT_ID=$SANDBOX_BLUEPRINT_ID
INSTANCE_BLUEPRINT_ID=$INSTANCE_BLUEPRINT_ID
TEE_INSTANCE_BLUEPRINT_ID=$TEE_INSTANCE_BLUEPRINT_ID

# Service IDs
SANDBOX_SERVICE_ID=$SANDBOX_SERVICE_ID
INSTANCE_SERVICE_ID=$INSTANCE_SERVICE_ID

# Operator
OPERATOR_API_PORT=$OPERATOR_API_PORT
SESSION_AUTH_SECRET=dev-secret-key-do-not-use-in-production
ALLOW_STANDALONE=true
CORS_ALLOWED_ORIGINS=*

# Accounts
DEPLOYER_KEY=$DEPLOYER_KEY
DEPLOYER_ADDR=$DEPLOYER_ADDR
OPERATOR1_KEY=$OPERATOR1_KEY
OPERATOR1_ADDR=$OPERATOR1_ADDR
OPERATOR2_KEY=$OPERATOR2_KEY
OPERATOR2_ADDR=$OPERATOR2_ADDR
USER_KEY=$USER_KEY
USER_ADDR=$USER_ADDR
EOF

# Write UI env if ui/ directory exists
if [ -d "$ROOT_DIR/ui" ]; then
    cat > "$ROOT_DIR/ui/.env.local" <<EOF
VITE_USE_LOCAL_CHAIN=true
VITE_RPC_URL=$RPC_URL
VITE_CHAIN_ID=31337
VITE_TANGLE_CONTRACT=$TANGLE
VITE_SANDBOX_BSM=$SANDBOX_BSM
VITE_INSTANCE_BSM=$INSTANCE_BSM
VITE_TEE_INSTANCE_BSM=$TEE_INSTANCE_BSM
VITE_SANDBOX_BLUEPRINT_ID=$SANDBOX_BLUEPRINT_ID
VITE_INSTANCE_BLUEPRINT_ID=$INSTANCE_BLUEPRINT_ID
VITE_TEE_INSTANCE_BLUEPRINT_ID=$TEE_INSTANCE_BLUEPRINT_ID
VITE_SANDBOX_SERVICE_ID=$SANDBOX_SERVICE_ID
VITE_INSTANCE_SERVICE_ID=$INSTANCE_SERVICE_ID
VITE_OPERATOR_API_URL=http://localhost:$OPERATOR_API_PORT
EOF
    echo "  Wrote ui/.env.local"
fi

echo ""
echo "=========================================================================="
echo "  AI Agent Sandbox — Local Testnet Ready"
echo "=========================================================================="
echo ""
echo "  Contracts:"
echo "    Tangle:          $TANGLE"
echo "    Sandbox BSM:     $SANDBOX_BSM  (blueprint #$SANDBOX_BLUEPRINT_ID)"
echo "    Instance BSM:    $INSTANCE_BSM  (blueprint #$INSTANCE_BLUEPRINT_ID)"
echo "    TEE Instance:    $TEE_INSTANCE_BSM  (blueprint #$TEE_INSTANCE_BLUEPRINT_ID)"
echo ""
echo "  Services:"
echo "    Sandbox:   service #$SANDBOX_SERVICE_ID  (2 operators, EventDriven)"
echo "    Instance:  service #$INSTANCE_SERVICE_ID  (2 operators, Subscription)"
echo ""
echo "  Operators:"
echo "    $OPERATOR1_ADDR → http://localhost:$OPERATOR_API_PORT"
echo "    $OPERATOR2_ADDR → http://localhost:$((OPERATOR_API_PORT + 1))"
echo ""
echo "  Accounts:"
echo "    Deployer: $DEPLOYER_ADDR"
echo "    User:     $USER_ADDR"
echo ""
echo "  API endpoints:"
echo "    GET  http://localhost:$OPERATOR_API_PORT/api/sandboxes"
echo "    GET  http://localhost:$OPERATOR_API_PORT/api/provisions"
echo "    POST http://localhost:$OPERATOR_API_PORT/api/auth/challenge"
echo "    POST http://localhost:$OPERATOR_API_PORT/api/auth/session"
echo ""
echo "  Press Ctrl+C to stop"
echo "=========================================================================="

wait
