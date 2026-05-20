# Blueprint Sidecar Image

This directory owns the public all-harness sidecar layer for the sandbox blueprint.

The sandbox runtime should not assume that an external floating image contains
every agent CLI. The image built here installs the harness toolchain in a
reviewable, reproducible place:

- Claude Code
- Codex
- opencode
- Kimi
- Gemini

The sidecar server base is required through `SIDECAR_BASE_IMAGE`. That keeps the
ownership boundary explicit: this repo owns the public harness layer, and it will
not silently fall back to a legacy sidecar image.

## Build

```bash
docker build -f sidecar/Dockerfile.all-harness \
  --build-arg SIDECAR_BASE_IMAGE="$BLUEPRINT_SIDECAR_SERVER_IMAGE" \
  -t ghcr.io/tangle-network/blueprint-sidecar:all-harness .
```

Build a smaller subset:

```bash
docker build -f sidecar/Dockerfile.all-harness \
  --build-arg SIDECAR_BASE_IMAGE="$BLUEPRINT_SIDECAR_SERVER_IMAGE" \
  --build-arg BLUEPRINT_HARNESSES=codex,gemini \
  -t ghcr.io/tangle-network/blueprint-sidecar:codex-gemini .
```

The publish workflow requires the repository variable
`BLUEPRINT_SIDECAR_SERVER_IMAGE`; without it, pull requests still validate the
harness layer, but main will not publish a fake complete runtime image.

## Verify

```bash
docker run --rm --entrypoint blueprint-verify-harnesses \
  ghcr.io/tangle-network/blueprint-sidecar:all-harness
```

## Local Profile

```bash
nix-shell sidecar/nix/harness-profile.nix
sh sidecar/scripts/install-all-harnesses.sh
```

Auth/config remains provider-specific and lives in the normal CLI directories:

- `/root/.claude`
- `/root/.codex`
- `/root/.kimi`
- `/root/.config/kimi`
- `/root/.gemini`
- `/root/.config/opencode`
- `/root/.opencode`
