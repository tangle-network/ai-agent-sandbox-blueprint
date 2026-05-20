{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  name = "blueprint-sidecar-harness-profile";

  packages = with pkgs; [
    bash
    cacert
    curl
    git
    nodejs_22
    python313
    uv
  ];

  shellHook = ''
    echo "blueprint sidecar harness profile"
    echo "Run: sh sidecar/scripts/install-harness.sh <claude|codex|opencode|kimi|gemini|all>"
    echo "Auth/config stays in each CLI's normal home directory."
  '';
}
