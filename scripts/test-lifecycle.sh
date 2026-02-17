#!/usr/bin/env bash
# test-lifecycle.sh — Exercise the full lifecycle against a running local stack.
#
# Usage:
#   # First, in another terminal:
#   ./scripts/deploy-local.sh
#
#   # Then run this script:
#   ./scripts/test-lifecycle.sh
#
# Environment overrides:
#   OPERATOR_API_URL   — Operator API base URL (default: http://localhost:9090)

set -euo pipefail

API="${OPERATOR_API_URL:-http://localhost:9090}"
PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
check() { if [ "$1" = "0" ]; then pass "$2"; else fail "$2"; fi; }

echo "=== Full Lifecycle Test ==="
echo "  API: $API"
echo ""

# -----------------------------------------------------------------------
# 1. Check API is reachable
# -----------------------------------------------------------------------
echo "[1] API Health..."
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/api/sandboxes")
if [ "$HTTP_CODE" = "200" ]; then
    pass "API reachable (GET /api/sandboxes → $HTTP_CODE)"
else
    fail "API not reachable (HTTP $HTTP_CODE)"
    echo "Is the stack running? Start it with: ./scripts/deploy-local.sh"
    exit 1
fi

# -----------------------------------------------------------------------
# 2. List sandboxes (should be empty or have existing ones)
# -----------------------------------------------------------------------
echo ""
echo "[2] Sandbox listing..."
SANDBOXES=$(curl -s "$API/api/sandboxes")
SANDBOX_COUNT=$(echo "$SANDBOXES" | jq '.sandboxes | length')
echo "  Current sandboxes: $SANDBOX_COUNT"
pass "GET /api/sandboxes"

# -----------------------------------------------------------------------
# 3. Provision progress
# -----------------------------------------------------------------------
echo ""
echo "[3] Provision progress..."
PROVISIONS=$(curl -s "$API/api/provisions")
PROVISION_COUNT=$(echo "$PROVISIONS" | jq '.provisions | length')
echo "  Current provisions: $PROVISION_COUNT"
pass "GET /api/provisions"

# Check 404 for nonexistent provision
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/api/provisions/999999999")
if [ "$HTTP_CODE" = "404" ]; then
    pass "GET /api/provisions/999999999 → 404"
else
    fail "Expected 404, got $HTTP_CODE"
fi

# -----------------------------------------------------------------------
# 4. Session auth flow
# -----------------------------------------------------------------------
echo ""
echo "[4] Session auth flow..."

# 4a: Request challenge
CHALLENGE=$(curl -s -X POST "$API/api/auth/challenge")
NONCE=$(echo "$CHALLENGE" | jq -r '.nonce')
MESSAGE=$(echo "$CHALLENGE" | jq -r '.message')
EXPIRES=$(echo "$CHALLENGE" | jq -r '.expires_at')

if [ -n "$NONCE" ] && [ "$NONCE" != "null" ] && [ ${#NONCE} -eq 64 ]; then
    pass "POST /api/auth/challenge (nonce=${NONCE:0:16}...)"
else
    fail "Challenge nonce invalid: $NONCE"
fi

# 4b: Try invalid signature (should fail)
INVALID_RESP=$(curl -s -X POST "$API/api/auth/session" \
    -H "Content-Type: application/json" \
    -d "{\"nonce\": \"$NONCE\", \"signature\": \"0xdeadbeef\"}")
INVALID_ERROR=$(echo "$INVALID_RESP" | jq -r '.error // empty')
if [ -n "$INVALID_ERROR" ]; then
    pass "Invalid signature rejected"
else
    fail "Invalid signature was not rejected"
fi

# 4c: Nonce consumed — replay should also fail
REPLAY_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/api/auth/session" \
    -H "Content-Type: application/json" \
    -d "{\"nonce\": \"$NONCE\", \"signature\": \"0xdeadbeef\"}")
if [ "$REPLAY_CODE" = "401" ]; then
    pass "Nonce replay rejected (401)"
else
    fail "Nonce replay not rejected (HTTP $REPLAY_CODE)"
fi

# -----------------------------------------------------------------------
# 5. CORS headers
# -----------------------------------------------------------------------
echo ""
echo "[5] CORS..."
CORS_HEADERS=$(curl -s -I -X OPTIONS "$API/api/sandboxes" \
    -H "Origin: http://localhost:5173" \
    -H "Access-Control-Request-Method: GET" 2>&1)
if echo "$CORS_HEADERS" | grep -qi "access-control-allow-origin"; then
    pass "CORS preflight returns allow-origin header"
else
    fail "Missing CORS headers"
fi

# -----------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------
echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
