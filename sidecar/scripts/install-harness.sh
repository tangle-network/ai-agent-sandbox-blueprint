#!/usr/bin/env sh
set -eu

harness="${1:-}"
prime_agent_version="${PRIME_AGENT_VERSION:-0.7.2}"
if [ -z "$harness" ]; then
  echo "usage: sidecar/scripts/install-harness.sh <claude|codex|opencode|kimi|gemini|prime|hermes|amp|factory-droids|pi|forge|openclaw|qwen|copilot|all>" >&2
  exit 2
fi

ensure_npm() {
  command -v npm >/dev/null 2>&1 || {
    echo "npm is required to install $1" >&2
    exit 1
  }
}

ensure_curl() {
  command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install $1" >&2
    exit 1
  }
}

copy_user_binary() {
  source_path="$1"
  destination_name="$2"
  if [ ! -x "$source_path" ]; then
    echo "installer did not produce executable $source_path" >&2
    exit 1
  fi
  mkdir -p /usr/local/bin
  cp "$source_path" "/usr/local/bin/$destination_name"
  chmod 0755 "/usr/local/bin/$destination_name"
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

install_uv() {
  if command -v uv >/dev/null 2>&1; then
    return
  fi
  command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install uv" >&2
    exit 1
  }
  curl -LsSf https://astral.sh/uv/install.sh | sh
  if [ -x "${HOME:-/root}/.local/bin/uv" ]; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    cp "${HOME:-/root}/.local/bin/uv" /usr/local/bin/uv 2>/dev/null \
      && chmod +x /usr/local/bin/uv 2>/dev/null \
      || true
  fi
  command -v uv >/dev/null 2>&1 || [ -x /usr/local/bin/uv ] || {
    echo "uv install did not produce a usable uv binary" >&2
    exit 1
  }
}

install_prime() {
  ensure_npm prime-agent
  install_uv
  command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install Prime Agent" >&2
    exit 1
  }
  curl -fsSL https://app.primeintellect.ai/prime-agent/install.sh \
    | PRIME_AGENT_INSTALLER_PLAIN=1 PRIME_AGENT_BOOTSTRAP_KERNEL_ON_INSTALL=0 \
      PRIME_AGENT_VERSION="$prime_agent_version" sh
  if command -v prime-agent >/dev/null 2>&1; then
    return
  fi
  npm_bin="$(npm bin -g 2>/dev/null || true)"
  if [ -n "$npm_bin" ] && [ -x "$npm_bin/prime-agent" ]; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    ln -sf "$npm_bin/prime-agent" /usr/local/bin/prime-agent 2>/dev/null || true
  fi
  command -v prime-agent >/dev/null 2>&1 || {
    echo "Prime Agent installer did not produce a prime-agent binary" >&2
    exit 1
  }
}

install_hermes() {
  command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install hermes" >&2
    exit 1
  }
  command -v bash >/dev/null 2>&1 || {
    echo "bash is required to install hermes" >&2
    exit 1
  }

  # The sidecar uses Hermes's Python CLI path. Skip npm lifecycle scripts in
  # the headless image because they only prepare browser/TUI tooling, which is
  # disabled here and can require native build steps unrelated to chat.
  hermes_install_env='NPM_CONFIG_IGNORE_SCRIPTS=true'
  if [ -n "${HERMES_INSTALL_COMMIT:-}" ]; then
    curl -fsSL https://hermes-agent.nousresearch.com/install.sh | \
      env "$hermes_install_env" bash -s -- \
      --skip-setup --skip-browser --skip-computer-use --no-skills \
      --non-interactive --commit "$HERMES_INSTALL_COMMIT"
  else
    curl -fsSL https://hermes-agent.nousresearch.com/install.sh | \
      env "$hermes_install_env" bash -s -- \
      --skip-setup --skip-browser --skip-computer-use --no-skills \
      --non-interactive
  fi

  if [ -x "${HOME:-/root}/.local/bin/hermes" ] && ! command -v hermes >/dev/null 2>&1; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    cp "${HOME:-/root}/.local/bin/hermes" /usr/local/bin/hermes 2>/dev/null \
      && chmod +x /usr/local/bin/hermes 2>/dev/null \
      || true
  fi
}

install_amp() {
  ensure_curl amp
  command -v bash >/dev/null 2>&1 || {
    echo "bash is required to install amp" >&2
    exit 1
  }
  amp_home="${HOME:-/root}/.amp"
  AMP_HOME="$amp_home" bash -c 'curl -fsSL https://ampcode.com/install.sh | bash'
  copy_user_binary "$amp_home/bin/amp" amp
}

install_factory_droids() {
  ensure_curl factory-droids
  factory_home="${HOME:-/root}/.local"
  curl -fsSL https://app.factory.ai/cli | sh
  copy_user_binary "$factory_home/bin/droid" droid
}

install_pi() {
  ensure_npm pi
  npm install -g @earendil-works/pi-coding-agent@latest
}

install_forge() {
  ensure_curl forge
  forge_home="${HOME:-/root}/.local"
  curl -fsSL https://forgecode.dev/cli | sh
  copy_user_binary "$forge_home/bin/forge" forgecode
}

install_openclaw() {
  ensure_npm openclaw
  npm install -g openclaw@latest
}

install_qwen() {
  ensure_npm qwen
  npm install -g @qwen-code/qwen-code@latest
}

install_copilot() {
  ensure_npm copilot
  npm install -g @github/copilot@latest
}

install_one() {
  case "$1" in
    claude) install_claude ;;
    codex) install_codex ;;
    opencode) install_opencode ;;
    kimi) install_kimi ;;
    gemini) install_gemini ;;
    prime) install_prime ;;
    hermes) install_hermes ;;
    amp) install_amp ;;
    factory-droids) install_factory_droids ;;
    pi) install_pi ;;
    forge) install_forge ;;
    openclaw) install_openclaw ;;
    qwen) install_qwen ;;
    copilot) install_copilot ;;
    *) echo "unknown harness: $1" >&2; exit 2 ;;
  esac
}

if [ "$harness" = "all" ]; then
  for name in claude codex opencode kimi gemini prime hermes amp factory-droids pi forge openclaw qwen copilot; do
    echo "==> installing $name"
    install_one "$name"
  done
else
  install_one "$harness"
fi
