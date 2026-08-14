{
  description = "Tangle blueprint sidecar harness profile";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    hermes-agent = {
      url = "github:NousResearch/hermes-agent/v2026.8.3";
      flake = true;
    };
    kimi-cli = {
      url = "github:MoonshotAI/kimi-cli/1.49.0";
      flake = true;
    };
  };

  outputs = { nixpkgs, hermes-agent, kimi-cli, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
      };
      binaries = import ./packages/binaries.nix { inherit pkgs; };
      npm = import ./packages/npm-with-deps.nix { inherit pkgs; };
      hermes = hermes-agent.packages.${system}.default;
      kimi = kimi-cli.packages.${system}.default;
      harnessPackages = [
        binaries.claude
        binaries.codex
        binaries.opencode
        kimi
        binaries.gemini
        npm.prime
        hermes
        binaries.amp
        binaries.factory
        npm.pi
        binaries.forge
        npm.openclaw
        binaries.qwen
        binaries.copilot
      ];
      supportPackages = with pkgs; [
        bash
        bun
        cacert
        curl
        git
        nodejs_24
        python313
        uv
      ];
      profile = pkgs.buildEnv {
        name = "blueprint-sidecar-harness-profile";
        paths = harnessPackages ++ supportPackages;
        pathsToLink = [ "/bin" ];
        ignoreCollisions = true;
      };
    in {
      packages.${system} = {
        default = profile;
        harness-profile = profile;
        claude = binaries.claude;
        codex = binaries.codex;
        opencode = binaries.opencode;
        inherit kimi hermes;
        gemini = binaries.gemini;
        prime = npm.prime;
        amp = binaries.amp;
        factory-droids = binaries.factory;
        pi = npm.pi;
        forge = binaries.forge;
        openclaw = npm.openclaw;
        qwen = binaries.qwen;
        copilot = binaries.copilot;
      };

      checks.${system}.harness-profile = pkgs.runCommand "verify-blueprint-harness-profile" {
        nativeBuildInputs = [ profile ];
      } ''
        export HOME="$TMPDIR/home"
        mkdir -p "$HOME"

        check_version() {
          binary="$1"
          expected="$2"
          echo "checking $binary ($expected)"
          command -v "$binary" >/dev/null
          output="$("$binary" --version 2>&1)"
          printf '%s\n' "$output"
          printf '%s\n' "$output" | grep -F "$expected" >/dev/null
        }

        check_version claude '2.1.232'
        check_version codex 'codex-cli 0.144.6'
        check_version opencode '1.18.18'
        check_version kimi '1.49.0'
        check_version gemini '0.55.1'
        check_version prime-agent '0.7.2'
        check_version hermes '0.20.0'
        check_version amp '0.0.1786651704-g574433'
        check_version droid '0.195.0'
        check_version pi '0.84.1'
        check_version forgecode 'forge 2.13.21'
        check_version openclaw 'OpenClaw 2026.7.1-2'
        check_version qwen '0.21.11'
        check_version copilot 'GitHub Copilot CLI 1.0.79'
        touch "$out"
      '';

      devShells.${system}.default = pkgs.mkShell {
        packages = [ profile ];
        shellHook = ''
          echo "blueprint sidecar profile: 14 agent CLIs"
        '';
      };
    };
}
