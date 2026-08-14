#!/usr/bin/env sh
set -eu

missing=""
for bin in bun claude codex opencode kimi gemini prime-agent uv hermes amp droid pi forgecode openclaw qwen copilot; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    missing="$missing $bin"
  fi
done

if [ -n "$missing" ]; then
  echo "missing harness binaries:$missing" >&2
  exit 1
fi

unusable=""
for bin in bun claude codex opencode kimi gemini prime-agent uv hermes amp droid pi forgecode openclaw qwen copilot; do
  if ! "$bin" --version >/dev/null 2>&1; then
    unusable="$unusable $bin"
  fi
done

if [ -n "$unusable" ]; then
  echo "unusable harness binaries:$unusable" >&2
  exit 1
fi

echo "all harness binaries are present and executable"
