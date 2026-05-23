#!/usr/bin/env bash
# Register the AI Agent Sandbox blueprint trio on Tangle.
#
# Deploys three `AgentSandboxBlueprint` BSMs (cloud, instance, tee-instance)
# AND calls `Tangle.createBlueprint` for each in the same broadcast via
# `contracts/script/RegisterBlueprint.s.sol`.
#
# Prerequisites:
#   - forge installed
#   - Deployer wallet funded on the target network
#
# Usage (Base Sepolia, against an already-deployed Tangle protocol):
#
#   export PRIVATE_KEY=0x...
#   export RPC_URL=https://sepolia.base.org
#   export TANGLE_CORE=0x8299d60f373F3A4a8C4878E335cb9d840e6E3730
#   export RESTAKING=0x91b1186f4f31d6e02e481c0af29c7244a3fe417d
#   ./deploy/register-blueprint.sh
#
# Local anvil (LocalTestnet snapshot):
#
#   export RPC_URL=http://127.0.0.1:8545
#   ./deploy/register-blueprint.sh   # uses anvil deployer + LocalTestnet defaults
#
# Outputs (parsed by `tnt-core/deploy/register-blueprints.sh`, do not change
# the format without coordinating):
#   DEPLOY_SANDBOX_BSM=<address>
#   DEPLOY_INSTANCE_BSM=<address>
#   DEPLOY_TEE_INSTANCE_BSM=<address>
#   DEPLOY_SANDBOX_BLUEPRINT_ID=<u64>
#   DEPLOY_INSTANCE_BLUEPRINT_ID=<u64>
#   DEPLOY_TEE_INSTANCE_BLUEPRINT_ID=<u64>

set -euo pipefail

: "${RPC_URL:?Set RPC_URL}"
: "${PRIVATE_KEY:?Set PRIVATE_KEY}"

echo "=== AI Agent Sandbox Blueprint Trio Registration ==="
echo "Network:     $(cast chain-id --rpc-url "$RPC_URL")"
echo "Deployer:    $(cast wallet address --private-key "$PRIVATE_KEY")"
echo "Tangle Core: ${TANGLE_CORE:-<LocalTestnet default>}"
echo "Restaking:   ${RESTAKING:-<LocalTestnet default>}"
echo ""

cd "$(dirname "$0")/../contracts"

# Single forge-script broadcast: 3× BSM deploys + 3× Tangle.createBlueprint.
DEPLOY_OUTPUT=$(PRIVATE_KEY="$PRIVATE_KEY" \
    TANGLE_CORE="${TANGLE_CORE:-}" \
    RESTAKING="${RESTAKING:-}" \
    forge script script/RegisterBlueprint.s.sol \
        --rpc-url "$RPC_URL" \
        --broadcast --slow)

echo "$DEPLOY_OUTPUT"

# Extract addresses + ids for downstream sweep scripts.
SANDBOX_BSM=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_SANDBOX_BSM=0x[0-9a-fA-F]+' | tail -1 | cut -d= -f2)
INSTANCE_BSM=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_INSTANCE_BSM=0x[0-9a-fA-F]+' | tail -1 | cut -d= -f2)
TEE_BSM=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_TEE_INSTANCE_BSM=0x[0-9a-fA-F]+' | tail -1 | cut -d= -f2)
SANDBOX_ID=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_SANDBOX_BLUEPRINT_ID=[0-9]+' | tail -1 | cut -d= -f2)
INSTANCE_ID=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_INSTANCE_BLUEPRINT_ID=[0-9]+' | tail -1 | cut -d= -f2)
TEE_ID=$(echo "$DEPLOY_OUTPUT" | grep -oE 'DEPLOY_TEE_INSTANCE_BLUEPRINT_ID=[0-9]+' | tail -1 | cut -d= -f2)

if [ -z "$SANDBOX_BSM" ] || [ -z "$SANDBOX_ID" ]; then
    echo "ERROR: failed to extract sandbox addresses from forge output" >&2
    exit 1
fi

echo ""
echo "=== Sandbox trio registered ==="
echo "Cloud sandbox  BSM=$SANDBOX_BSM   blueprintId=$SANDBOX_ID"
echo "Instance       BSM=$INSTANCE_BSM  blueprintId=$INSTANCE_ID"
echo "TEE instance   BSM=$TEE_BSM       blueprintId=$TEE_ID"
echo ""

# The sweep's manifest parser (in tnt-core/deploy/register-blueprints.sh)
# captures the LAST DEPLOY_*_BLUEPRINT_ID via `tail -1`. Print the cloud
# sandbox id last so the sweep records the canonical one — the other two
# are still recoverable from this script's full stdout.
echo "DEPLOY_SANDBOX_BSM=$SANDBOX_BSM"
echo "DEPLOY_SANDBOX_BLUEPRINT_ID=$SANDBOX_ID"
