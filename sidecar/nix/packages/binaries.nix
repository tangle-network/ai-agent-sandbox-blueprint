{ pkgs }:

let
  lib = pkgs.lib;
  glibcLibraryPath = lib.makeLibraryPath [
    pkgs.glibc
    pkgs.stdenv.cc.cc.lib
    pkgs.openssl
    pkgs.zlib
  ];

  installGlibcCli = binary: source: ''
    mkdir -p "$out/bin" "$out/libexec"
    install -Dm755 "${source}" "$out/libexec/${binary}"
    cat > "$out/bin/${binary}" <<EOF
    #!${pkgs.runtimeShell}
    exec ${pkgs.glibc}/lib/ld-linux-x86-64.so.2 \
      --library-path ${glibcLibraryPath} \
      "$out/libexec/${binary}" "\$@"
    EOF
    chmod 0755 "$out/bin/${binary}"
  '';

  mkNodeCli = {
    name,
    binary,
    version,
    url,
    hash,
    entry,
  }: pkgs.stdenvNoCC.mkDerivation {
    pname = name;
    inherit version;
    src = pkgs.fetchurl { inherit url hash; };
    nativeBuildInputs = [ pkgs.makeWrapper ];
    sourceRoot = "package";
    unpackPhase = "tar xzf $src";
    installPhase = ''
      mkdir -p "$out/lib/${name}" "$out/bin"
      cp -r . "$out/lib/${name}/"
      makeWrapper ${pkgs.nodejs_24}/bin/node "$out/bin/${binary}" \
        --add-flags "$out/lib/${name}/${entry}"
    '';
    meta = {
      mainProgram = binary;
      license = lib.licenses.unfree;
      platforms = [ "x86_64-linux" ];
    };
  };

  mkGlibcCli = {
    name,
    binary ? name,
    version,
    url,
    hash,
  }: pkgs.stdenvNoCC.mkDerivation {
    pname = name;
    inherit version;
    src = pkgs.fetchurl { inherit url hash; };
    dontUnpack = true;
    dontPatchELF = true;
    dontStrip = true;
    installPhase = installGlibcCli binary "$src";
    meta = {
      mainProgram = binary;
      license = lib.licenses.unfree;
      platforms = [ "x86_64-linux" ];
    };
  };
in {
  claude = pkgs.stdenvNoCC.mkDerivation {
    pname = "claude-code";
    version = "2.1.232";
    src = pkgs.fetchurl {
      url = "https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.232.tgz";
      hash = "sha512-nEd0W44UKPPgFeXLjWl9Qikyieowi8BFABdbYtyL1Cq05pw62h312M9ErCLjRj1cvWeMOYY+tg8RL5npjSQNSg==";
    };
    sourceRoot = "package";
    unpackPhase = "tar xzf $src";
    dontPatchELF = true;
    dontStrip = true;
    installPhase = installGlibcCli "claude" "claude";
    meta = {
      mainProgram = "claude";
      license = lib.licenses.unfree;
      platforms = [ "x86_64-linux" ];
    };
  };

  codex = pkgs.stdenvNoCC.mkDerivation {
    pname = "codex";
    version = "0.144.6";
    src = pkgs.fetchurl {
      url = "https://registry.npmjs.org/@openai/codex/-/codex-0.144.6-linux-x64.tgz";
      hash = "sha512-4E7EnzCg0OnBxCyYnwJ+qnZwWHYe0YScr5ucKWbngE9u4+0XrpWELqq2Kn9jl5GZK8MDjU7PrJwFIwusHOHjuw==";
    };
    nativeBuildInputs = [ pkgs.makeWrapper ];
    sourceRoot = "package";
    unpackPhase = "tar xzf $src";
    installPhase = ''
      codex_root="$out/lib/codex/vendor/x86_64-unknown-linux-musl"
      mkdir -p "$codex_root" "$out/bin"
      cp -r vendor/x86_64-unknown-linux-musl/* "$codex_root/"
      makeWrapper "$codex_root/bin/codex" "$out/bin/codex" \
        --prefix PATH : "$codex_root/codex-path:$codex_root/codex-resources:$codex_root/codex-resources/zsh/bin"
    '';
    meta = {
      mainProgram = "codex";
      license = lib.licenses.asl20;
      platforms = [ "x86_64-linux" ];
    };
  };

  opencode = pkgs.stdenvNoCC.mkDerivation {
    pname = "opencode";
    version = "1.18.18";
    src = pkgs.fetchurl {
      url = "https://github.com/anomalyco/opencode/releases/download/v1.18.18/opencode-linux-x64.tar.gz";
      hash = "sha256-DN3CIkGLhVNmmQWomAwM2nCI8A2iTYPWrHawHJ/bKq8=";
    };
    dontPatchELF = true;
    dontStrip = true;
    sourceRoot = ".";
    unpackPhase = "tar xzf $src";
    installPhase = installGlibcCli "opencode" "opencode";
    meta = {
      mainProgram = "opencode";
      license = lib.licenses.mit;
      platforms = [ "x86_64-linux" ];
    };
  };

  gemini = mkNodeCli {
    name = "gemini-cli";
    binary = "gemini";
    version = "0.55.1";
    url = "https://registry.npmjs.org/@google/gemini-cli/-/gemini-cli-0.55.1.tgz";
    hash = "sha512-leEv91V7J3YWhZdXqYIj4nTl0hXl8oNos5aVR0whPCFqVbRvoFPTzaQOHdI2UIT1wGgp+XdCi4qUrFDnUFN7RQ==";
    entry = "bundle/gemini.js";
  };

  amp = mkGlibcCli {
    name = "amp";
    version = "0.0.1786651704-g574433";
    url = "https://static.ampcode.com/cli/0.0.1786651704-g574433/amp-linux-x64";
    hash = "sha256-T7hOdat52UezIHG27vVwFfvY4uo3BfFAoZl0tyi+0KM=";
  };

  factory = mkGlibcCli {
    name = "factory-droids";
    binary = "droid";
    version = "0.195.0";
    url = "https://downloads.factory.ai/factory-cli/releases/0.195.0/linux/x64/droid";
    hash = "sha256-bf4097xDB4TEgytHaEVMit1L7Je01Cpm88nO+b+wWOM=";
  };

  qwen = mkNodeCli {
    name = "qwen-code";
    binary = "qwen";
    version = "0.21.11";
    url = "https://registry.npmjs.org/@qwen-code/qwen-code/-/qwen-code-0.21.11.tgz";
    hash = "sha512-0UIAcIw3zXdh0BUJZBClUkiJ7JkMte0Mfe1mIA3ZVQ3KrGp6R2VGioYoSD3przcTduCivdp7Sk06Geeh8K/kKw==";
    entry = "cli.js";
  };

  copilot = pkgs.stdenvNoCC.mkDerivation {
    pname = "github-copilot-cli";
    version = "1.0.79";
    src = pkgs.fetchurl {
      url = "https://registry.npmjs.org/@github/copilot-linux-x64/-/copilot-linux-x64-1.0.79.tgz";
      hash = "sha512-wzotZfvHkItutciLFMXZT2k9Qiii4Ta8tsVDCMQ7CP8hPxV91FyJ1yf3+FFSSfPvWrfYM6BOAiqIuX+LjgRuiw==";
    };
    dontPatchELF = true;
    dontStrip = true;
    sourceRoot = "package";
    unpackPhase = "tar xzf $src";
    installPhase = installGlibcCli "copilot" "copilot";
    meta = {
      mainProgram = "copilot";
      license = lib.licenses.unfree;
      platforms = [ "x86_64-linux" ];
    };
  };

  forge = pkgs.stdenvNoCC.mkDerivation {
    pname = "forgecode";
    version = "2.13.21";
    src = pkgs.fetchurl {
      url = "https://github.com/tailcallhq/forgecode/releases/download/v2.13.21/forge-x86_64-unknown-linux-musl";
      hash = "sha256-SubYbN0AHmSeG0NdmSQ9cknRaeMLJFKeNQ+d/seY8po=";
    };
    dontUnpack = true;
    dontPatchELF = true;
    dontStrip = true;
    installPhase = ''
      install -Dm755 "$src" "$out/bin/forgecode"
    '';
    meta = {
      mainProgram = "forgecode";
      license = lib.licenses.asl20;
      platforms = [ "x86_64-linux" ];
    };
  };
}
