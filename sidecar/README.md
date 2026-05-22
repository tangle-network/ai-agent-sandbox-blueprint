# Blueprint Sidecar Image

This directory owns the public all-harness sidecar runtime for the sandbox blueprint.

The sandbox runtime should not assume that an external floating image contains
the sidecar server or every agent CLI. The image built here ships the sidecar
HTTP contract plus the harness toolchain in a reviewable, reproducible place:

- Claude Code
- Codex
- opencode
- Kimi
- Gemini

## Build

```bash
docker build -f sidecar/Dockerfile.all-harness \
  -t ghcr.io/tangle-network/blueprint-sidecar:all-harness .
```

Build a smaller subset:

```bash
docker build -f sidecar/Dockerfile.all-harness \
  --build-arg BLUEPRINT_HARNESSES=codex,gemini \
  -t ghcr.io/tangle-network/blueprint-sidecar:codex-gemini .
```

## Publish

The GitHub Actions workflow publishes the runtime image to GHCR:

- `ghcr.io/tangle-network/blueprint-sidecar:all-harness` — stable moving tag for local dev and blueprint defaults.
- `ghcr.io/tangle-network/blueprint-sidecar:all-harness-<git-sha>` — immutable commit tag for reproducible deployments.
- `ghcr.io/tangle-network/blueprint-sidecar:all-harness-<release-tag>` — release/tag alias when publishing a GitHub Release or pushing a matching tag.

Manual publish is available from the `Sidecar Image` workflow. Publishing a
GitHub Release also creates release-specific aliases such as
`all-harness-v1.2.3` and `all-harness-1.2.3`.

The workflow prunes old GHCR versions after successful publishes:

- keeps the stable/release tags;
- keeps the newest 20 SHA-only versions by default;
- keeps the newest 5 untagged versions;
- deletes older SHA-only and untagged package versions.

## Verify

```bash
docker run --rm --entrypoint blueprint-verify-harnesses \
  ghcr.io/tangle-network/blueprint-sidecar:all-harness
```

## Local Cleanup

Local Docker caches are independent of GHCR retention. To remove old local
copies without touching the current stable tag:

```bash
docker images 'ghcr.io/tangle-network/blueprint-sidecar' --format '{{.Repository}}:{{.Tag}} {{.ID}}' \
  | awk '$1 ~ /:all-harness-[0-9a-f]{40}$/ {print $2}' \
  | sort -u \
  | xargs -r docker rmi
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
