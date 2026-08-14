# Blueprint Sidecar Image

This directory owns the public all-harness sidecar runtime for the sandbox blueprint.

The sandbox runtime should not assume that an external floating image contains
the sidecar server or every agent CLI. The image built here ships the sidecar
HTTP contract plus the harness toolchain in one reviewable, version-pinned place:

- Claude Code
- Codex
- opencode
- Kimi
- Gemini
- Prime Agent
- Hermes Agent (Nous Research)
- AMP
- Factory Droids
- Pi
- ForgeCode
- OpenClaw
- Qwen Code
- GitHub Copilot CLI

## Runtime Boundary

[Exo](https://github.com/exoharness/exo) is not a sidecar harness entry.
Exo is a stateful orchestration runtime, not a one-shot agent CLI.
It should integrate above this process adapter through its socket and workspace contracts.

## Build

```bash
docker build -f sidecar/Dockerfile.all-harness \
  -t ghcr.io/tangle-network/blueprint-sidecar:all-harness .
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
nix develop ./sidecar/nix
```

Build the same profile for a `/nix/profile` mount:

```bash
nix build ./sidecar/nix#harness-profile
```

The flake packages all 14 CLIs in the Nix store.
Its lock file pins Nixpkgs, Hermes, Kimi, and every transitive package source.
Byte-exact native CLIs run through the pinned Nix glibc without modifying their embedded bundles.

Auth/config remains provider-specific and lives in the normal CLI directories:

- `/home/agent/.claude`
- `/home/agent/.codex`
- `/home/agent/.kimi`
- `/home/agent/.config/kimi`
- `/home/agent/.gemini`
- `/home/agent/.config/opencode`
- `/home/agent/.opencode`
- `/home/agent/.prime/agent` for Prime Agent config and credentials
- `/home/agent/.hermes` for Hermes state and credentials
- `/home/agent/.config/amp`
- `/home/agent/.factory`
- `/home/agent/.forge`
- `/home/agent/.openclaw`
- `/home/agent/.pi`
- `/home/agent/.qwen`
- `/home/agent/.copilot`

Prime Agent runs with its official `-p` print contract.
The sidecar keeps the persistent Prime Agent config at
`/home/agent/.prime/agent` and passes the official `--no-session` flag.
Model credentials remain in the persistent config or environment variables such
as `PRIME_API_KEY`.
The image install pins Prime Agent `0.7.2`; update its package URL and checksum
together in `sidecar/scripts/install-harness.sh`.

The sidecar sets `HERMES_HOME=/home/agent/.hermes` before every Hermes run, so
sessions and state use one persistent location.
It passes `--provider` when the request selects one and always passes `--yolo`
because sidecar calls have no interactive approval channel.
The install script uses Nous Research's official installer with setup,
browser, computer-use, and bundled skills disabled for a headless image.
The installer checksum, source archive checksum, and upstream commit are pinned in
`sidecar/scripts/install-harness.sh`.
