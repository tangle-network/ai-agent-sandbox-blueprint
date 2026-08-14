#!/usr/bin/env sh
set -eu

harness="${1:-}"

# These values come from the image smoke builds and the corresponding
# release/package metadata captured on 2026-08-13. Update the value and its
# checksum together in one reviewed change.
claude_code_version='2.1.232'
claude_code_integrity='sha512-Lmt5bfOrGmnI0A3vvuvYHrbKtINqJMn3a5k+4yiaWXoTwiCkZKIypO+bAT/hVxvnIO/+6QDgMSEUvSy/K+PB+g=='
codex_version='0.144.6'
codex_integrity='sha512-wk+2CWiBNXiJLBoN2D08N9RceWkSBnlgk5g2K1a4CXrP/C0gdlHyRUG7RFzm9y41DCK/7tvCct233JVxyFmznw=='
gemini_version='0.55.1'
gemini_integrity='sha512-leEv91V7J3YWhZdXqYIj4nTl0hXl8oNos5aVR0whPCFqVbRvoFPTzaQOHdI2UIT1wGgp+XdCi4qUrFDnUFN7RQ=='
pi_version='0.84.1'
pi_integrity='sha512-ncAqFrG+iybuPGOhMiZoEHkEzTpJgz3guYD32pD+M7ucc0WeHmauP6wa7qwP8V/KWvsZDVNa5XGsdZ7fkC7w7A=='
openclaw_version='2026.7.1-2'
openclaw_integrity='sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g=='
qwen_version='0.21.11'
qwen_integrity='sha512-0UIAcIw3zXdh0BUJZBClUkiJ7JkMte0Mfe1mIA3ZVQ3KrGp6R2VGioYoSD3przcTduCivdp7Sk06Geeh8K/kKw=='
copilot_version='1.0.79'
copilot_integrity='sha512-uHBm2BYbKJgyfiKp1WokX7QUNHGvzEX0zaGeb3qM3CybP06rsJrX3JgQe95qwwma6vQz0ah9gV68ERW2JqaKRA=='
kimi_version='1.49.0'
kimi_wheel_sha256='3a0ed632bed97f8bf05403309ea3823031051ca27264ff6d5f2b2b01bc90976e'
kimi_wheel_url='https://files.pythonhosted.org/packages/31/44/677c07fcefb99bf28eefed1248fddc4880801576dd5aee911c9e6492c265/kimi_cli-1.49.0-py3-none-any.whl'
prime_agent_version='0.7.2'
prime_agent_package_sha256='bc5471f2a626d727b88a45eb745fff93b10c554a3c4fc5912f25d8c64b987f5e'
prime_agent_package_url='https://pub-728493de92a943e2a9b2d17b4719f318.r2.dev/releases/v0.7.2/prime-agent-0.7.2.tgz'
hermes_install_commit='8c8d55bd07575604a76f6df59bfbb42ceb6a71e6'
hermes_installer_sha256='458ed1873bec1766ccd723b8a86338fbdf1caff5d43eae45065bc448cafa2dca'
hermes_source_archive_sha256='69f7deec70680ccd6e9c7830acc0dadf6b7d195c63a544d671c02dad8a784a1c'
amp_version='0.0.1786651704-g574433'
amp_binary_sha256='4fb84e75ab79d947b32071b6eef57015fbd8e2ea3705f140a19974b728bed0a3'
factory_droids_version='0.195.0'
factory_droids_binary_sha256='6dfe34f7bc430784c4832b4768454c8add4bec97b4d42a66f3c9cef9bfb058e3'
forge_version='2.13.21'
forge_binary_sha256='4ae6d86cdd001e649e1b435d99243d7249d169e30b24529e350f9dfec798f29a'
opencode_version='1.18.18'
opencode_archive_sha256='0cddc222418b8553669905a8980c0cda7088f00da24d83d6ac76b01c9fdb2aaf'
uv_version='0.12.4'
uv_archive_sha256='c8c60f47e6f88d18dbf6f33d7279fb1fbf7ae76631768152cf5578c3d65729b4'
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

verify_sha256() {
  expected="$1"
  path="$2"
  actual="$(sha256sum "$path" | awk '{print $1}')"
  if [ "$actual" != "$expected" ]; then
    echo "sha256 mismatch for $path: expected $expected, got $actual" >&2
    exit 1
  fi
}

verify_sha512_integrity() {
  integrity="$1"
  path="$2"
  expected="$(printf '%s' "${integrity#sha512-}" | base64 -d | od -An -tx1 | tr -d ' \n')"
  actual="$(sha512sum "$path" | awk '{print $1}')"
  if [ "$actual" != "$expected" ]; then
    echo "sha512 integrity mismatch for $path: expected $expected, got $actual" >&2
    exit 1
  fi
}

download_verified() {
  url="$1"
  expected_sha256="$2"
  destination="$3"
  curl --fail --show-error --silent --location \
    --connect-timeout 15 --max-time 300 \
    --retry 3 --retry-delay 2 --retry-all-errors \
    "$url" -o "$destination"
  verify_sha256 "$expected_sha256" "$destination"
}

install_npm_package() {
  package="$1"
  version="$2"
  integrity="$3"
  package_dir="$(mktemp -d)"
  tarball_name="$(npm pack --silent --ignore-scripts --pack-destination "$package_dir" "$package@$version")"
  tarball="$package_dir/$(basename "$tarball_name")"
  verify_sha512_integrity "$integrity" "$tarball"
  npm install -g --no-fund --no-audit "$tarball"
  rm -rf "$package_dir"
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
  install_npm_package '@anthropic-ai/claude-code' "$claude_code_version" "$claude_code_integrity"
}

install_codex() {
  ensure_npm codex
  install_npm_package '@openai/codex' "$codex_version" "$codex_integrity"
}

install_opencode() {
  ensure_curl opencode
  opencode_dir="$(mktemp -d)"
  opencode_archive="$opencode_dir/opencode.tar.gz"
  download_verified \
    "https://github.com/anomalyco/opencode/releases/download/v$opencode_version/opencode-linux-x64.tar.gz" \
    "$opencode_archive_sha256" "$opencode_archive"
  tar -xzf "$opencode_archive" -C "$opencode_dir"
  copy_user_binary "$opencode_dir/opencode" opencode
  rm -rf "$opencode_dir"
}

install_kimi() {
  install_uv
  uv_bin="$(command -v uv || true)"
  if [ -z "$uv_bin" ] && [ -x "${HOME:-/root}/.local/bin/uv" ]; then
    uv_bin="${HOME:-/root}/.local/bin/uv"
  fi
  if [ -z "$uv_bin" ]; then
    echo "uv install did not produce a uv binary on PATH or ~/.local/bin" >&2
    exit 1
  fi
  kimi_dir="$(mktemp -d)"
  kimi_wheel="$kimi_dir/kimi_cli-$kimi_version-py3-none-any.whl"
  download_verified "$kimi_wheel_url" "$kimi_wheel_sha256" "$kimi_wheel"
  mkdir -p /home/agent/.local/share/uv/tools /home/agent/.local/bin
  UV_TOOL_DIR=/home/agent/.local/share/uv/tools \
    UV_TOOL_BIN_DIR=/usr/local/bin \
    UV_PYTHON_INSTALL_DIR=/home/agent/.local/share/uv/python \
    "$uv_bin" tool install --python 3.13 "$kimi_wheel"
  rm -rf "$kimi_dir"
  command -v kimi >/dev/null 2>&1 || {
    echo "Kimi installer did not produce a kimi binary" >&2
    exit 1
  }
}

install_gemini() {
  ensure_npm gemini
  install_npm_package '@google/gemini-cli' "$gemini_version" "$gemini_integrity"
}

install_uv() {
  if command -v uv >/dev/null 2>&1 && uv --version | grep -q " $uv_version "; then
    return
  fi
  ensure_curl uv
  uv_dir="$(mktemp -d)"
  uv_archive="$uv_dir/uv.tar.gz"
  download_verified \
    "https://github.com/astral-sh/uv/releases/download/$uv_version/uv-x86_64-unknown-linux-gnu.tar.gz" \
    "$uv_archive_sha256" "$uv_archive"
  tar -xzf "$uv_archive" -C "$uv_dir"
  mkdir -p /usr/local/bin
  cp "$uv_dir/uv-x86_64-unknown-linux-gnu/uv" /usr/local/bin/uv
  cp "$uv_dir/uv-x86_64-unknown-linux-gnu/uvx" /usr/local/bin/uvx
  chmod 0755 /usr/local/bin/uv /usr/local/bin/uvx
  rm -rf "$uv_dir"
}

install_prime() {
  ensure_npm prime-agent
  install_uv
  prime_dir="$(mktemp -d)"
  prime_package="$prime_dir/prime-agent-$prime_agent_version.tgz"
  download_verified "$prime_agent_package_url" "$prime_agent_package_sha256" "$prime_package"
  npm install -g --no-fund --no-audit "$prime_package"
  rm -rf "$prime_dir"
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
  ensure_curl hermes
  command -v bash >/dev/null 2>&1 || {
    echo "bash is required to install hermes" >&2
    exit 1
  }

  # Hermes otherwise downloads Astral's floating uv installer. Seed its
  # managed path with the same verified uv artifact used by Prime and Kimi.
  install_uv
  hermes_home="${HOME:-/root}/.hermes"
  mkdir -p "$hermes_home/bin"
  cp "$(command -v uv)" "$hermes_home/bin/uv"
  chmod 0755 "$hermes_home/bin/uv"

  # Use the official installer stages against an exact source archive. The
  # installer's repository stage clones a large branch before applying the
  # commit pin, which makes container builds slow and unbounded.
  hermes_install_env='NPM_CONFIG_IGNORE_SCRIPTS=true'
  hermes_dir="$(mktemp -d)"
  hermes_installer="$hermes_dir/install.sh"
  hermes_source="$hermes_dir/hermes-agent.tar.gz"
  hermes_install_dir='/usr/local/lib/hermes-agent'
  download_verified "https://hermes-agent.nousresearch.com/install.sh" "$hermes_installer_sha256" "$hermes_installer"
  download_verified \
    "https://github.com/NousResearch/hermes-agent/archive/$hermes_install_commit.tar.gz" \
    "$hermes_source_archive_sha256" "$hermes_source"
  mkdir -p "$hermes_install_dir"
  tar -xzf "$hermes_source" -C "$hermes_install_dir" --strip-components=1

  for stage in venv path config; do
    if [ "$stage" = path ]; then
      (
        cd "$hermes_install_dir"
        UV_PROJECT_ENVIRONMENT="$hermes_install_dir/venv" \
          UV_PYTHON="$hermes_install_dir/venv/bin/python" \
          "$hermes_home/bin/uv" sync --extra all --frozen
      )
    fi
    env "$hermes_install_env" HERMES_HOME="$hermes_home" bash "$hermes_installer" \
      --dir "$hermes_install_dir" --hermes-home "$hermes_home" \
      --skip-setup --skip-browser --skip-computer-use --no-skills \
      --non-interactive --stage "$stage"
  done
  printf '%s\n' "$hermes_install_commit" > "$hermes_install_dir/.source_commit"
  rm -rf "$hermes_dir"

  if [ -x "${HOME:-/root}/.local/bin/hermes" ] && ! command -v hermes >/dev/null 2>&1; then
    mkdir -p /usr/local/bin 2>/dev/null || true
    cp "${HOME:-/root}/.local/bin/hermes" /usr/local/bin/hermes 2>/dev/null \
      && chmod +x /usr/local/bin/hermes 2>/dev/null \
      || true
  fi
  installed_hermes_commit="$(cat "$hermes_install_dir/.source_commit" 2>/dev/null || true)"
  [ "$installed_hermes_commit" = "$hermes_install_commit" ] || {
    echo "Hermes source archive produced unexpected commit $installed_hermes_commit" >&2
    exit 1
  }
}

install_amp() {
  ensure_curl amp
  amp_config='/home/agent/.config/amp'
  amp_dir="$(mktemp -d)"
  amp_binary="$amp_dir/amp"
  download_verified \
    "https://static.ampcode.com/cli/$amp_version/amp-linux-x64" \
    "$amp_binary_sha256" "$amp_binary"
  mkdir -p "$amp_config"
  chmod 0755 "$amp_binary"
  copy_user_binary "$amp_binary" amp
  rm -rf "$amp_dir"
}

install_factory_droids() {
  ensure_curl factory-droids
  factory_home="${HOME:-/root}/.local"
  factory_dir="$(mktemp -d)"
  factory_binary="$factory_dir/droid"
  download_verified \
    "https://downloads.factory.ai/factory-cli/releases/$factory_droids_version/linux/x64/droid" \
    "$factory_droids_binary_sha256" "$factory_binary"
  mkdir -p "$factory_home/bin"
  cp "$factory_binary" "$factory_home/bin/droid"
  chmod 0755 "$factory_home/bin/droid"
  copy_user_binary "$factory_home/bin/droid" droid
  rm -rf "$factory_dir"
}

install_pi() {
  ensure_npm pi
  install_npm_package '@earendil-works/pi-coding-agent' "$pi_version" "$pi_integrity"
}

install_forge() {
  ensure_curl forge
  forge_dir="$(mktemp -d)"
  forge_binary="$forge_dir/forgecode"
  download_verified \
    "https://github.com/tailcallhq/forgecode/releases/download/v$forge_version/forge-x86_64-unknown-linux-musl" \
    "$forge_binary_sha256" "$forge_binary"
  chmod 0755 "$forge_binary"
  copy_user_binary "$forge_binary" forgecode
  rm -rf "$forge_dir"
}

install_openclaw() {
  ensure_npm openclaw
  install_npm_package openclaw "$openclaw_version" "$openclaw_integrity"
}

install_qwen() {
  ensure_npm qwen
  install_npm_package '@qwen-code/qwen-code' "$qwen_version" "$qwen_integrity"
}

install_copilot() {
  ensure_npm copilot
  install_npm_package '@github/copilot' "$copilot_version" "$copilot_integrity"
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
