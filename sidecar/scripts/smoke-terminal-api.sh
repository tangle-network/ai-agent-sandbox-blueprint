#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
port="${SIDECAR_SMOKE_PORT:-$(FORCE_COLOR=0 node -e "const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{process.stdout.write(String(s.address().port));s.close();})")}"
token="${SIDECAR_SMOKE_TOKEN:-sidecar-terminal-smoke}"
workspace="$(mktemp -d)"
log_file="$(mktemp)"
stream_file="$(mktemp)"
sidecar_pid=""

cleanup() {
  if [ -n "${sidecar_pid}" ] && kill -0 "${sidecar_pid}" 2>/dev/null; then
    kill "${sidecar_pid}" 2>/dev/null || true
    wait "${sidecar_pid}" 2>/dev/null || true
  fi
  rm -rf "${workspace}"
  rm -f "${log_file}" "${stream_file}"
}
trap cleanup EXIT

SIDECAR_PORT="${port}" \
SIDECAR_AUTH_TOKEN="${token}" \
AGENT_WORKSPACE_ROOT="${workspace}" \
node "${repo_root}/sidecar/server/server.js" >"${log_file}" 2>&1 &
sidecar_pid="$!"

for _ in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:${port}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

curl -fsS "http://127.0.0.1:${port}/health" >/dev/null

terminal_json="$(
  curl -fsS \
    -H "authorization: Bearer ${token}" \
    -H 'content-type: application/json' \
    -d '{"title":"Smoke","cols":100,"rows":30}' \
    "http://127.0.0.1:${port}/terminals"
)"

terminal_id="$(
  node -e "const body=JSON.parse(process.argv[1]); console.log(body.sessionId || body.session_id || body.data?.sessionId || body.data?.session_id || '')" \
    "${terminal_json}"
)"

if [ -z "${terminal_id}" ]; then
  echo "terminal smoke failed: no terminal id returned" >&2
  cat "${log_file}" >&2
  exit 1
fi

curl -fsS \
  -H "authorization: Bearer ${token}" \
  -H 'content-type: application/json' \
  -d '{"data":"echo terminal-smoke-ok\n"}' \
  "http://127.0.0.1:${port}/terminals/${terminal_id}/input" >/dev/null

for _ in $(seq 1 10); do
  : >"${stream_file}"
  timeout 1s curl -N -fsS \
    -H "authorization: Bearer ${token}" \
    "http://127.0.0.1:${port}/terminals/${terminal_id}/stream" >"${stream_file}" || true
  if grep -q 'terminal-smoke-ok' "${stream_file}"; then
    break
  fi
  sleep 0.2
done

if ! grep -q 'terminal-smoke-ok' "${stream_file}"; then
  echo "terminal smoke failed: stream did not contain terminal-smoke-ok" >&2
  cat "${stream_file}" >&2
  cat "${log_file}" >&2
  exit 1
fi

curl -fsS \
  -H "authorization: Bearer ${token}" \
  "http://127.0.0.1:${port}/terminals" \
  | node -e "const fs=require('fs');const body=JSON.parse(fs.readFileSync(0,'utf8'));const list=body.data||body.terminals||[]; if(!list.some((item)=>item.sessionId==='${terminal_id}'||item.session_id==='${terminal_id}')) process.exit(1)"

curl -fsS \
  -X DELETE \
  -H "authorization: Bearer ${token}" \
  "http://127.0.0.1:${port}/terminals/${terminal_id}" >/dev/null

echo "sidecar terminal smoke passed"
