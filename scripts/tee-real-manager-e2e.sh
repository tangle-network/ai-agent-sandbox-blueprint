#!/usr/bin/env bash
set -euo pipefail

# Production-style TEE instance fulfillment check.
#
# This script intentionally does not deploy contracts, start Anvil, or launch
# blueprint binaries in test mode. It exercises the path a real operator uses:
# deployed Tangle manager -> registered remote operator -> TEE instance service
# request -> operator direct report -> caller-authenticated nonce attestation.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZERO_ADDR="0x0000000000000000000000000000000000000000"
ZERO_BYTES32="0x0000000000000000000000000000000000000000000000000000000000000000"
PROVISION_TIMEOUT_SECONDS="${PROVISION_TIMEOUT_SECONDS:-900}"
PROVISION_POLL_SECONDS="${PROVISION_POLL_SECONDS:-10}"
REQUEST_TTL_SECONDS="${REQUEST_TTL_SECONDS:-31536000}"
SERVICE_MEMBERSHIP_MODEL="${SERVICE_MEMBERSHIP_MODEL:-1}"
APPROVAL_PERCENT="${APPROVAL_PERCENT:-100}"
GAS_LIMIT="${GAS_LIMIT:-10000000}"
TEE_TYPE_ID="${TEE_TYPE_ID:-2}"
TEE_BACKEND="${TEE_BACKEND:-nitro}"
SERVICE_NAME="${SERVICE_NAME:-prod-tee-sandbox}"
SIDECAR_IMAGE="${SIDECAR_IMAGE:-ghcr.io/tangle-network/agent-dev-container-sidecar:latest}"
STACK="${STACK:-default}"
AGENT_IDENTIFIER="${AGENT_IDENTIFIER:-default-agent}"
ENV_JSON="${ENV_JSON:-{}}"
METADATA_JSON="${METADATA_JSON:-{}}"
SSH_ENABLED="${SSH_ENABLED:-false}"
SSH_PUBLIC_KEY="${SSH_PUBLIC_KEY:-}"
MAX_LIFETIME_SECONDS="${MAX_LIFETIME_SECONDS:-3600}"
IDLE_TIMEOUT_SECONDS="${IDLE_TIMEOUT_SECONDS:-900}"
CPU_CORES="${CPU_CORES:-2}"
MEMORY_MB="${MEMORY_MB:-4096}"
DISK_GB="${DISK_GB:-20}"

usage() {
    cat <<'EOF'
Usage:
  RPC_URL=... \
  TANGLE_CONTRACT=... \
  TEE_INSTANCE_BLUEPRINT_ID=... \
  TEE_INSTANCE_BSM=... \
  USER_KEY=... \
  OPERATOR_KEY=... \
  OPERATOR_RPC_ENDPOINT=https://operator.example.com \
  TEE_OPERATOR_API_URL=https://operator.example.com \
  scripts/tee-real-manager-e2e.sh

Required:
  RPC_URL                    Deployed Tangle EVM RPC URL
  TANGLE_CONTRACT            Deployed Tangle service manager/precompile address
  TEE_INSTANCE_BLUEPRINT_ID  Registered ai-agent-tee-instance blueprint id
  TEE_INSTANCE_BSM           Deployed TEE instance Blueprint Service Manager
  USER_KEY                   Customer private key requesting the service
  OPERATOR_KEY               Remote operator private key, unless SKIP_REGISTER_OPERATOR=1 and
                             SKIP_APPROVE_SERVICE=1 are both set
  OPERATOR_RPC_ENDPOINT      Public endpoint registered for the operator, unless
                             SKIP_REGISTER_OPERATOR=1
  TEE_OPERATOR_API_URL       Operator API URL used to fetch attestation

Optional:
  SKIP_REGISTER_OPERATOR=1       Reuse an already registered operator
  SKIP_CONFIGURE_PRICING=1       Do not run ConfigureJobRates.s.sol
  SKIP_REQUEST_SERVICE=1         Reuse SERVICE_ID instead of requesting a service
  SKIP_APPROVE_SERVICE=1         Do not approve the service request
  SERVICE_ID=...                 Existing service id when SKIP_REQUEST_SERVICE=1
  TEE_ATTESTATION_NONCE=0x...    32-64 byte nonce; generated if omitted
  VERIFY_ATTESTATION_CMD='...'   Command run with ATTESTATION_JSON, ATTESTATION_NONCE,
                                 TEE_BACKEND, SERVICE_ID, SANDBOX_ID exported
  ALLOW_LOCAL=1                  Permit localhost URLs for rehearsal only

This script fails if the operator does not report a TEE attestation hash on-chain
and if it cannot fetch a nonce-bound attestation artifact from the operator API.
EOF
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

log() {
    echo "==> $*"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_env() {
    local name="$1"
    [[ -n "${!name:-}" ]] || die "missing required env var: $name"
}

normalize_uint() {
    local value
    value="$(echo "${1:-0}" | awk '{print $1}')"
    value="${value#0x}"
    value="$(echo "$value" | sed 's/^0*//')"
    [[ -n "$value" ]] || value="0"
    echo "$value"
}

is_local_url() {
    local value="$1"
    [[ "$value" =~ ^https?://(localhost|127\.0\.0\.1|0\.0\.0\.0)(:|/|$) ]]
}

post_json() {
    local url="$1"
    local body="$2"
    shift 2
    curl -fsS "$url" \
        -H 'content-type: application/json' \
        "$@" \
        --data "$body"
}

get_json() {
    local url="$1"
    shift
    curl -fsS "$url" "$@"
}

derive_address() {
    cast wallet address --private-key "$1" | awk '{print $1}'
}

derive_pubkey_for_registration() {
    local raw
    raw="$(cast wallet public-key --private-key "$1" | head -1)"
    raw="${raw#0x}"
    if [[ "$raw" == 04* ]]; then
        echo "0x$raw"
    else
        echo "0x04$raw"
    fi
}

request_session_token() {
    local challenge_json message nonce signature session_json

    challenge_json="$(post_json "$TEE_OPERATOR_API_URL/api/auth/challenge" '{}')"
    nonce="$(jq -er '.nonce' <<<"$challenge_json")"
    message="$(jq -er '.message' <<<"$challenge_json")"
    signature="$(cast wallet sign --private-key "$USER_KEY" "$message" | awk '{print $1}')"
    session_json="$(post_json "$TEE_OPERATOR_API_URL/api/auth/session" "$(jq -nc --arg nonce "$nonce" --arg signature "$signature" '{nonce:$nonce, signature:$signature}')")"
    jq -er '.token' <<<"$session_json"
}

discover_service_id() {
    local before="$1"
    local after="$2"
    local service_id data word blueprint_num

    if (( after <= before )); then
        return 1
    fi

    for service_id in $(seq "$before" "$((after - 1))"); do
        data="$(cast call "$TANGLE_CONTRACT" "getService(uint64)" "$service_id" --rpc-url "$RPC_URL" 2>/dev/null || true)"
        [[ -n "$data" ]] || continue
        word="$(echo "$data" | tr -d '\n' | head -c 66)"
        blueprint_num="$(normalize_uint "$word")"
        if [[ "$blueprint_num" == "$TEE_INSTANCE_BLUEPRINT_ID" ]]; then
            echo "$service_id"
            return 0
        fi
    done

    return 1
}

poll_operator_sandbox() {
    local service_id="$1"
    local token="$2"
    local deadline now provision_json sandbox_id
    deadline=$(( $(date +%s) + PROVISION_TIMEOUT_SECONDS ))

    while true; do
        sandbox_id="$(
            get_json "$TEE_OPERATOR_API_URL/api/sandboxes" -H "authorization: Bearer $token" \
                | jq -er --arg sid "$service_id" '
                    [
                        .sandboxes[]?
                        | select(((.service_id // .serviceId // "") | tostring) == $sid)
                        | select((.tee_deployment_id // .teeDeploymentId // "") != "")
                        | .id
                    ][0]
                ' 2>/dev/null || true
        )"
        if [[ -n "$sandbox_id" && "$sandbox_id" != "null" ]]; then
            echo "$sandbox_id"
            return 0
        fi

        provision_json="$(get_json "$TEE_OPERATOR_API_URL/api/provisions" || true)"
        if [[ -n "$provision_json" ]]; then
            sandbox_id="$(
                jq -er --arg sid "$service_id" '
                    [
                        .provisions[]?
                        | select(
                            ((.service_id // .serviceId // .service // "") | tostring) == $sid
                            or ((.metadata.service_id // .metadata.serviceId // "") | tostring) == $sid
                          )
                        | select(((.phase // .status // "") | tostring | ascii_downcase) == "ready")
                        | (.sandbox_id // .sandboxId // .metadata.sandbox_id // .metadata.sandboxId // empty)
                    ][0]
                ' <<<"$provision_json" 2>/dev/null || true
            )"
            if [[ -n "$sandbox_id" && "$sandbox_id" != "null" ]]; then
                echo "$sandbox_id"
                return 0
            fi
        fi

        now="$(date +%s)"
        if (( now >= deadline )); then
            return 1
        fi
        sleep "$PROVISION_POLL_SECONDS"
    done
}

check_attestation_payload() {
    local file="$1"
    jq -e '
        .sandbox_id as $sandbox
        | .attestation as $att
        | ($sandbox | type == "string" and length > 0)
        and ($att | type == "object")
        and (
            ($att.evidence? | type == "array" and length > 0)
            or ($att.evidence? | type == "string" and length > 0)
            or ($att.evidence? | type == "object" and length > 0)
            or ($att.quote? | type == "string" and length > 0)
            or ($att.raw_quote? | type == "string" and length > 0)
            or ($att.document? | type == "string" and length > 0)
        )
    ' "$file" >/dev/null
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

for cmd in cast curl jq openssl sed awk date; do
    require_cmd "$cmd"
done

require_env RPC_URL
require_env TANGLE_CONTRACT
require_env TEE_INSTANCE_BLUEPRINT_ID
require_env TEE_INSTANCE_BSM
require_env USER_KEY
require_env TEE_OPERATOR_API_URL

if [[ "${SKIP_REGISTER_OPERATOR:-0}" != "1" ]]; then
    require_env OPERATOR_RPC_ENDPOINT
fi

if [[ "${SKIP_REQUEST_SERVICE:-0}" == "1" ]]; then
    require_env SERVICE_ID
fi

if [[ "${SKIP_REGISTER_OPERATOR:-0}" != "1" || "${SKIP_APPROVE_SERVICE:-0}" != "1" ]]; then
    require_env OPERATOR_KEY
fi

if [[ "${ALLOW_LOCAL:-0}" != "1" ]]; then
    is_local_url "$RPC_URL" && die "RPC_URL points at localhost; set ALLOW_LOCAL=1 only for rehearsal"
    if [[ -n "${OPERATOR_RPC_ENDPOINT:-}" ]]; then
        is_local_url "$OPERATOR_RPC_ENDPOINT" && die "OPERATOR_RPC_ENDPOINT points at localhost; set ALLOW_LOCAL=1 only for rehearsal"
    fi
    is_local_url "$TEE_OPERATOR_API_URL" && die "TEE_OPERATOR_API_URL points at localhost; set ALLOW_LOCAL=1 only for rehearsal"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

USER_ADDR="${USER_ADDR:-$(derive_address "$USER_KEY")}"
if [[ -n "${OPERATOR_KEY:-}" ]]; then
    OPERATOR_ADDR="${OPERATOR_ADDR:-$(derive_address "$OPERATOR_KEY")}"
else
    require_env OPERATOR_ADDR
fi

if [[ -z "${TEE_ATTESTATION_NONCE:-}" ]]; then
    TEE_ATTESTATION_NONCE="0x$(openssl rand -hex 32)"
fi
[[ "$TEE_ATTESTATION_NONCE" =~ ^0x[0-9a-fA-F]{64}([0-9a-fA-F]{64})?$ ]] \
    || die "TEE_ATTESTATION_NONCE must be a 32-64 byte hex string"

log "User:     $USER_ADDR"
log "Operator: $OPERATOR_ADDR"
log "TEE:      backend=$TEE_BACKEND type=$TEE_TYPE_ID nonce=$TEE_ATTESTATION_NONCE"

if [[ "${SKIP_CONFIGURE_PRICING:-0}" != "1" ]]; then
    require_cmd forge
    if [[ -n "${PRICING_KEY:-${OPERATOR_KEY:-}}" ]]; then
        log "Configuring TEE blueprint job rates"
        BASE_RATE="${BASE_RATE:-1000000000000000}" \
        BLUEPRINT_ID="$TEE_INSTANCE_BLUEPRINT_ID" \
        TANGLE_ADDRESS="$TANGLE_CONTRACT" \
        BSM_ADDRESS="$TEE_INSTANCE_BSM" \
        forge script "$ROOT_DIR/contracts/script/ConfigureJobRates.s.sol:ConfigureJobRates" \
            --rpc-url "$RPC_URL" \
            --private-key "${PRICING_KEY:-$OPERATOR_KEY}" \
            --broadcast >/dev/null
    else
        die "pricing requires PRICING_KEY or OPERATOR_KEY; set SKIP_CONFIGURE_PRICING=1 to skip"
    fi
else
    log "Skipping pricing configuration"
fi

if [[ "${SKIP_REGISTER_OPERATOR:-0}" != "1" ]]; then
    OPERATOR_PUBKEY="$(derive_pubkey_for_registration "$OPERATOR_KEY")"
    log "Registering remote operator for TEE blueprint"
    if ! cast send "$TANGLE_CONTRACT" \
        "registerOperator(uint64,bytes,string)" \
        "$TEE_INSTANCE_BLUEPRINT_ID" \
        "$OPERATOR_PUBKEY" \
        "$OPERATOR_RPC_ENDPOINT" \
        --gas-limit "$GAS_LIMIT" \
        --rpc-url "$RPC_URL" \
        --private-key "$OPERATOR_KEY" >/dev/null; then
        log "registerOperator failed; continuing only if already registered"
    fi
fi

OPERATOR_REGISTERED="$(cast call "$TANGLE_CONTRACT" \
    "isOperatorRegistered(uint64,address)(bool)" \
    "$TEE_INSTANCE_BLUEPRINT_ID" \
    "$OPERATOR_ADDR" \
    --rpc-url "$RPC_URL" | awk '{print $1}')"
[[ "$OPERATOR_REGISTERED" == "true" ]] || die "operator is not registered for TEE blueprint"

if [[ "${SKIP_REQUEST_SERVICE:-0}" != "1" ]]; then
    SERVICE_COUNT_BEFORE="$(normalize_uint "$(cast call "$TANGLE_CONTRACT" "serviceCount()(uint64)" --rpc-url "$RPC_URL")")"
    REQUEST_ID="$(normalize_uint "$(cast call "$TANGLE_CONTRACT" "serviceRequestCount()(uint64)" --rpc-url "$RPC_URL")")"
    SERVICE_CONFIG="$(cast abi-encode \
        "f(string,string,string,string,string,string,bool,string,bool,uint64,uint64,uint64,uint64,uint64,bool,uint8,string)" \
        "$SERVICE_NAME" "$SIDECAR_IMAGE" "$STACK" "$AGENT_IDENTIFIER" "$ENV_JSON" "$METADATA_JSON" \
        "$SSH_ENABLED" "$SSH_PUBLIC_KEY" false \
        "$MAX_LIFETIME_SECONDS" "$IDLE_TIMEOUT_SECONDS" "$CPU_CORES" "$MEMORY_MB" "$DISK_GB" \
        true "$TEE_TYPE_ID" "$TEE_ATTESTATION_NONCE")"

    log "Requesting TEE instance service request_id=$REQUEST_ID"
    cast send "$TANGLE_CONTRACT" \
        "requestService(uint64,address[],bytes,address[],uint64,address,uint256,uint8)" \
        "$TEE_INSTANCE_BLUEPRINT_ID" \
        "[$OPERATOR_ADDR]" \
        "$SERVICE_CONFIG" \
        "[$USER_ADDR]" \
        "$REQUEST_TTL_SECONDS" \
        "$ZERO_ADDR" \
        0 \
        "$SERVICE_MEMBERSHIP_MODEL" \
        --gas-limit "$GAS_LIMIT" \
        --rpc-url "$RPC_URL" \
        --private-key "$USER_KEY" >/dev/null

    if [[ "${SKIP_APPROVE_SERVICE:-0}" != "1" ]]; then
        log "Approving TEE service request as operator"
        cast send "$TANGLE_CONTRACT" \
            "approveService(uint64,uint8)" \
            "$REQUEST_ID" \
            "$APPROVAL_PERCENT" \
            --gas-limit "$GAS_LIMIT" \
            --rpc-url "$RPC_URL" \
            --private-key "$OPERATOR_KEY" >/dev/null
    fi

    SERVICE_COUNT_AFTER="$(normalize_uint "$(cast call "$TANGLE_CONTRACT" "serviceCount()(uint64)" --rpc-url "$RPC_URL")")"
    SERVICE_ID="$(discover_service_id "$SERVICE_COUNT_BEFORE" "$SERVICE_COUNT_AFTER")" \
        || die "could not discover TEE service id in service range $SERVICE_COUNT_BEFORE..$((SERVICE_COUNT_AFTER - 1))"
else
    log "Reusing existing TEE service id $SERVICE_ID"
fi

log "Waiting for on-chain TEE operator report for service_id=$SERVICE_ID"
DEADLINE=$(( $(date +%s) + PROVISION_TIMEOUT_SECONDS ))
while true; do
    PROVISIONED="$(cast call "$TEE_INSTANCE_BSM" \
        "isOperatorProvisioned(uint64,address)(bool)" \
        "$SERVICE_ID" \
        "$OPERATOR_ADDR" \
        --rpc-url "$RPC_URL" | awk '{print $1}' || true)"
    ATTESTATION_HASH="$(cast call "$TEE_INSTANCE_BSM" \
        "getAttestationHash(uint64,address)(bytes32)" \
        "$SERVICE_ID" \
        "$OPERATOR_ADDR" \
        --rpc-url "$RPC_URL" | awk '{print $1}' || true)"

    if [[ "$PROVISIONED" == "true" && "$ATTESTATION_HASH" =~ ^0x[0-9a-fA-F]{64}$ && "$ATTESTATION_HASH" != "$ZERO_BYTES32" ]]; then
        break
    fi

    if (( $(date +%s) >= DEADLINE )); then
        die "operator did not report provisioned TEE service with non-empty attestation hash"
    fi
    sleep "$PROVISION_POLL_SECONDS"
done

log "On-chain report accepted: attestation_hash=$ATTESTATION_HASH"

log "Authenticating to operator API as requester"
SESSION_TOKEN="$(request_session_token)"

SANDBOX_ID="${SANDBOX_ID:-$(poll_operator_sandbox "$SERVICE_ID" "$SESSION_TOKEN" || true)}"
if [[ -z "$SANDBOX_ID" ]]; then
    die "could not find TEE sandbox_id for service_id=$SERVICE_ID from authenticated operator API"
fi

ATTESTATION_JSON="$TMP_DIR/tee-attestation.json"
log "Fetching nonce-bound attestation for sandbox_id=$SANDBOX_ID"
post_json \
    "$TEE_OPERATOR_API_URL/api/sandboxes/$SANDBOX_ID/tee/attestation" \
    "$(jq -nc --arg nonce "$TEE_ATTESTATION_NONCE" '{attestation_nonce:$nonce}')" \
    -H "authorization: Bearer $SESSION_TOKEN" > "$ATTESTATION_JSON"

check_attestation_payload "$ATTESTATION_JSON" \
    || die "operator returned malformed or empty attestation payload: $ATTESTATION_JSON"

PERSISTED_ATTESTATION_JSON="$PWD/tee-real-manager-e2e-attestation.json"
cp "$ATTESTATION_JSON" "$PERSISTED_ATTESTATION_JSON"

if [[ -n "${VERIFY_ATTESTATION_CMD:-}" ]]; then
    log "Running caller-side attestation verifier"
    export ATTESTATION_JSON
    export ATTESTATION_NONCE="$TEE_ATTESTATION_NONCE"
    export TEE_BACKEND
    export SERVICE_ID
    export SANDBOX_ID
    bash -c "$VERIFY_ATTESTATION_CMD"
else
    log "VERIFY_ATTESTATION_CMD not set; saved evidence for external verifier"
fi

RESULT_JSON="$PWD/tee-real-manager-e2e-result.json"
jq -n \
    --arg service_id "$SERVICE_ID" \
    --arg sandbox_id "$SANDBOX_ID" \
    --arg operator "$OPERATOR_ADDR" \
    --arg attestation_hash "$ATTESTATION_HASH" \
    --arg nonce "$TEE_ATTESTATION_NONCE" \
    --arg tee_backend "$TEE_BACKEND" \
    --arg attestation_json "$PERSISTED_ATTESTATION_JSON" \
    '{
        service_id: $service_id,
        sandbox_id: $sandbox_id,
        operator: $operator,
        attestation_hash: $attestation_hash,
        nonce: $nonce,
        tee_backend: $tee_backend,
        attestation_json: $attestation_json
    }' > "$RESULT_JSON"

log "TEE manager e2e passed"
log "Result: $RESULT_JSON"
