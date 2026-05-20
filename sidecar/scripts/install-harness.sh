#!/usr/bin/env sh
set -eu

harness="${1:-}"
if [ -z "$harness" ]; then
  echo "usage: sidecar/scripts/install-harness.sh <claude|codex|opencode|kimi|gemini|all>" >&2
  exit 2
fi

ensure_npm() {
  command -v npm >/dev/null 2>&1 || {
    echo "npm is required to install $1" >&2
    exit 1
  }
}

install_claude() {
  ensure_npm claude
  npm install -g @anthropic-ai/claude-code@latest
}

install_codex() {
  ensure_npm codex
  npm install -g @openai/codex@latest
}

install_opencode() {
  command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install opencode" >&2
    exit 1
  }
  command -v bash >/dev/null 2>&1 || {
    echo "bash is required to install opencode" >&2
    exit 1
  }
  curl -fsSL https://opencode.ai/install | bash
  if [ -x "${HOME:-/root}/.opencode/bin/opencode" ]; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    cp "${HOME:-/root}/.opencode/bin/opencode" /usr/local/bin/opencode 2>/dev/null \
      && chmod +x /usr/local/bin/opencode 2>/dev/null \
      || true
  fi
}

install_kimi() {
  if ! command -v uv >/dev/null 2>&1; then
    command -v curl >/dev/null 2>&1 || {
      echo "curl is required to install uv/kimi" >&2
      exit 1
    }
    curl -LsSf https://astral.sh/uv/install.sh | sh
  fi
  uv_bin="$(command -v uv || true)"
  if [ -z "$uv_bin" ] && [ -x "${HOME:-/root}/.local/bin/uv" ]; then
    uv_bin="${HOME:-/root}/.local/bin/uv"
  fi
  if [ -z "$uv_bin" ]; then
    echo "uv install did not produce a uv binary on PATH or ~/.local/bin" >&2
    exit 1
  fi
  "$uv_bin" tool install --python 3.13 kimi-cli
  if [ -x "${HOME:-/root}/.local/bin/kimi" ]; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    ln -sf "${HOME:-/root}/.local/bin/kimi" /usr/local/bin/kimi 2>/dev/null || true
    ln -sf "$uv_bin" /usr/local/bin/uv 2>/dev/null || true
  fi
}

install_gemini() {
  ensure_npm gemini
  npm install -g @google/gemini-cli@latest
}

install_one() {
  case "$1" in
    claude) install_claude ;;
    codex) install_codex ;;
    opencode) install_opencode ;;
    kimi) install_kimi ;;
    gemini) install_gemini ;;
    *) echo "unknown harness: $1" >&2; exit 2 ;;
  esac
}

if [ "$harness" = "all" ]; then
  for name in claude codex opencode kimi gemini; do
    echo "==> installing $name"
    install_one "$name"
  done
else
  install_one "$harness"
fi
