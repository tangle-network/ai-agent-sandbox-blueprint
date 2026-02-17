#!/usr/bin/env bash
# deploy-local.sh — Start Anvil + deploy contracts + run operator binary.
#
# Usage:
#   ./scripts/deploy-local.sh
#
# Prerequisites:
#   - cargo, forge, cast, anvil (Foundry toolchain)
#   - Docker (for sidecar containers)
#   - A local sidecar image: docker build -t tangle-sidecar:local <sidecar-repo>
#
# Environment overrides:
#   ANVIL_PORT         — Anvil RPC port (default: 8545)
#   OPERATOR_API_PORT  — Operator REST API port (default: 9090)
#   SIDECAR_IMAGE      — Docker image for sidecars (default: tangle-sidecar:local)
#   RUST_LOG           — Log level (default: info)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration
ANVIL_PORT="${ANVIL_PORT:-8545}"
OPERATOR_API_PORT="${OPERATOR_API_PORT:-9090}"
SIDECAR_IMAGE="${SIDECAR_IMAGE:-tangle-sidecar:local}"

# Anvil deterministic private keys (account index 0 = deployer, 1 = operator, 2 = user)
DEPLOYER_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
OPERATOR_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
USER_KEY="0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"

cleanup() {
    echo ""
    echo "Shutting down..."
    [ -n "${ANVIL_PID:-}" ] && kill "$ANVIL_PID" 2>/dev/null || true
    [ -n "${OPERATOR_PID:-}" ] && kill "$OPERATOR_PID" 2>/dev/null || true
    exit 0
}
trap cleanup INT TERM

echo "=== AI Agent Sandbox Blueprint — Local Deployment ==="
echo ""

# 1. Start Anvil
echo "[1/5] Starting Anvil on port $ANVIL_PORT..."
anvil --block-time 2 --port "$ANVIL_PORT" --silent &
ANVIL_PID=$!
sleep 2

# Verify Anvil is running
if ! cast block-number --rpc-url "http://localhost:$ANVIL_PORT" >/dev/null 2>&1; then
    echo "ERROR: Anvil not responding on port $ANVIL_PORT"
    exit 1
fi
echo "  Anvil running (PID: $ANVIL_PID)"

# 2. Deploy contracts
echo ""
echo "[2/5] Deploying contracts..."
cd "$ROOT_DIR/contracts"

DEPLOY_OUTPUT=$(forge script script/Deploy.s.sol \
    --rpc-url "http://localhost:$ANVIL_PORT" \
    --broadcast \
    --private-key "$DEPLOYER_KEY" \
    2>&1) || {
    echo "ERROR: Contract deployment failed"
    echo "$DEPLOY_OUTPUT"
    exit 1
}

# Extract deployed address from forge output
BLUEPRINT_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -oP 'deployed at: \K0x[a-fA-F0-9]+' | head -1)
if [ -z "$BLUEPRINT_ADDRESS" ]; then
    echo "WARNING: Could not extract blueprint address from deployment output"
    BLUEPRINT_ADDRESS="0x0000000000000000000000000000000000000000"
fi
echo "  Blueprint deployed at: $BLUEPRINT_ADDRESS"
cd "$ROOT_DIR"

# 3. Build the blueprint binary
echo ""
echo "[3/5] Building blueprint binary..."
cargo build --release -p ai-agent-sandbox-blueprint-bin 2>&1 | tail -5
echo "  Binary built"

# 4. Set environment and start the operator
echo ""
echo "[4/5] Starting operator..."

export HTTP_RPC_ENDPOINT="http://localhost:$ANVIL_PORT"
export WS_RPC_ENDPOINT="ws://localhost:$ANVIL_PORT"
export BLUEPRINT_ID=0
export SERVICE_ID=0
export OPERATOR_API_PORT
export SIDECAR_IMAGE
export SIDECAR_PULL_IMAGE=false
export SIDECAR_PUBLIC_HOST="127.0.0.1"
export REQUEST_TIMEOUT_SECS=60
export SESSION_AUTH_SECRET="dev-secret-key-do-not-use-in-production"
export RUST_LOG="${RUST_LOG:-info}"

# Keystore setup (use Anvil's deterministic operator key)
export KEYSTORE_URI="file:///tmp/blueprint-keystore"
mkdir -p /tmp/blueprint-keystore

./target/release/ai-agent-sandbox-blueprint-bin run &
OPERATOR_PID=$!

# 5. Wait for operator API to be ready
echo ""
echo "[5/5] Waiting for operator API..."
DEADLINE=$((SECONDS + 30))
until curl -sf "http://localhost:$OPERATOR_API_PORT/api/sandboxes" > /dev/null 2>&1; do
    if [ $SECONDS -ge $DEADLINE ]; then
        echo "WARNING: Operator API not ready within 30s (operator may still be starting)"
        break
    fi
    sleep 1
done

echo ""
echo "=========================================="
echo "  Stack ready!"
echo ""
echo "  Anvil:          http://localhost:$ANVIL_PORT"
echo "  Operator API:   http://localhost:$OPERATOR_API_PORT"
echo "  Blueprint addr: $BLUEPRINT_ADDRESS"
echo ""
echo "  Deployer key:   $DEPLOYER_KEY"
echo "  Operator key:   $OPERATOR_KEY"
echo "  User key:       $USER_KEY"
echo ""
echo "  API endpoints:"
echo "    GET  /api/sandboxes"
echo "    GET  /api/provisions"
echo "    GET  /api/provisions/{call_id}"
echo "    POST /api/auth/challenge"
echo "    POST /api/auth/session"
echo ""
echo "  Press Ctrl+C to stop"
echo "=========================================="

wait
