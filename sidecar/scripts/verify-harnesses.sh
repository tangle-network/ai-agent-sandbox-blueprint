#!/usr/bin/env sh
set -eu

missing=""
for bin in bun claude codex opencode kimi gemini; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    missing="$missing $bin"
  fi
done

if [ -n "$missing" ]; then
  echo "missing harness binaries:$missing" >&2
  exit 1
fi

echo "all harness binaries are present"
