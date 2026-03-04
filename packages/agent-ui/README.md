# @tangle-network/agent-ui

Shared agent-facing UI package for Tangle apps.

## Scope

Put code in `@tangle-network/agent-ui` when it is:
- Agent chat/session UX (`ChatContainer`, run timeline, tool previews)
- Agent markdown rendering and tool/result presentation
- Sidecar auth/session orchestration and PTY terminal integration
- Reusable agent-focused primitives/hooks (`@tangle-network/agent-ui/primitives`)

Do not put code here when it is:
- Chain/contract/provisioning infra (belongs in `@tangle-network/blueprint-ui`)
- Product-specific route composition, copy, or business workflows (belongs in the app)
- Sandbox-only shell/layout styling concerns (belongs in the sandbox app)

## Public API

Entrypoints:
- `@tangle-network/agent-ui`: main agent components/hooks/types
- `@tangle-network/agent-ui/primitives`: small shared helpers/hooks for consumer UIs
- `@tangle-network/agent-ui/terminal`: lazy terminal view entry
- `@tangle-network/agent-ui/styles`: package stylesheet

Treat exported symbols as stable contract; prefer additive changes over breaking renames/removals.

## Installation

```bash
npm install @tangle-network/agent-ui
# or
pnpm add @tangle-network/agent-ui
```

## Usage

```tsx
import { ChatContainer, useWagmiSidecarAuth } from '@tangle-network/agent-ui';
import { copyText, timeAgo } from '@tangle-network/agent-ui/primitives';

const TerminalView = React.lazy(() =>
  import('@tangle-network/agent-ui/terminal').then((m) => ({ default: m.TerminalView })),
);
```

## Extraction Rule

If Sandbox and Arena share substantial agent-facing code (roughly 20+ lines), extract it here instead of duplicating app-local logic.

## Release

- Publish target: npm package `@tangle-network/agent-ui`
- Workflow: `.github/workflows/publish-agent-ui.yml`
- Triggers:
  - Push tag `agent-ui-vX.Y.Z` (must match `packages/agent-ui/package.json` version)
  - Manual `workflow_dispatch` with `version` input (must match package version)
- Publish auth: npm Trusted Publishing (OIDC), no long-lived npm token required once configured

## Repo Strategy

`@tangle-network/agent-ui` is a cross-product package for shared agent experience building blocks.

Keep it in this repo while:
1. Most API changes are driven by sandbox-runtime sidecar integration work.
2. Release cadence matches this monorepo.

Split to its own repo when:
1. There are 3+ external consumers with independent release cadence.
2. Separate ownership/review boundaries are needed.
3. Most changes no longer depend on this monorepo internals.

## Consumer Checklist

Before merging changes that touch agent UX:
1. Check if the same pattern exists in both UIs.
2. If yes and app-agnostic, move it into `@tangle-network/agent-ui`.
3. Keep exports explicit in `src/index.ts` or `src/primitives.ts`.
4. Verify both consumers still typecheck/build after the change.
