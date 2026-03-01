# @tangle/agent-ui

Shared agent-facing UI package for Tangle apps.

## Scope

Put code in `@tangle/agent-ui` when it is:
- Agent chat/session UX (`ChatContainer`, run timeline, tool previews)
- Agent markdown rendering and tool/result presentation
- Sidecar auth/session orchestration and PTY terminal integration
- Reusable agent-focused primitives/hooks (`@tangle/agent-ui/primitives`)

Do not put code here when it is:
- Chain/contract/provisioning infra (belongs in `@tangle/blueprint-ui`)
- Product-specific route composition, copy, or business workflows (belongs in the app)

## Public API

Entrypoints:
- `@tangle/agent-ui`: main agent components/hooks/types
- `@tangle/agent-ui/primitives`: small shared helpers/hooks for consumer UIs
- `@tangle/agent-ui/terminal`: lazy terminal view entry
- `@tangle/agent-ui/styles`: package stylesheet

Treat exported symbols as stable contract; prefer additive changes over breaking renames/removals.

## Usage

```tsx
import { ChatContainer, useWagmiSidecarAuth } from '@tangle/agent-ui';
import { copyText, timeAgo } from '@tangle/agent-ui/primitives';

const TerminalView = React.lazy(() =>
  import('@tangle/agent-ui/terminal').then((m) => ({ default: m.TerminalView })),
);
```

## Extraction Rule

If Sandbox and Arena share substantial agent-facing code (roughly 20+ lines), extract it here instead of duplicating app-local logic.

## Consumer Checklist

Before merging changes that touch agent UX:
1. Check if the same pattern exists in both UIs.
2. If yes and app-agnostic, move it into `@tangle/agent-ui`.
3. Keep exports explicit in `src/index.ts` or `src/primitives.ts`.
4. Verify both consumers still typecheck/build after the change.
