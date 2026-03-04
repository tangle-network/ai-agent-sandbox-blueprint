# Project Instructions

## Mission
`@tangle-network/agent-ui` is a shared package for agent-facing UX primitives and flows. Treat it as a reusable library, not an app.

## Scope
Include:
- Chat/session rendering and run timeline UX
- Tool preview and markdown presentation components
- Sidecar auth/session hooks and PTY terminal integration
- Reusable agent-centric helpers/hooks for consuming apps

Exclude:
- Chain/contract/provisioning logic (`@tangle/blueprint-ui`)
- Product-specific route orchestration, copy, and workflow logic (consumer app)
- Direct imports from any consuming app source tree

## Public API Discipline
- `src/index.ts`, `src/primitives.ts`, and `src/terminal.ts` are the package contract.
- Prefer additive API evolution; avoid breaking renames/removals without migration.
- Keep types explicit and avoid `any` in exported surfaces.

## Shared-Extraction Rule
- If Sandbox and Arena share agent-facing code above roughly 20 lines, extract it here.
- If duplication is chain/infra oriented, extract to `@tangle/blueprint-ui` instead.
- Keep app-specific behavior in the app when reuse would require brittle abstractions.

## Dependency Rules
- Keep runtime dependencies lean and justified.
- Use peer dependencies for framework/runtime packages where possible.
- Avoid introducing app-specific assumptions into package internals.

## Quality Gate
Before merging:
1. Confirm code belongs in package scope.
2. Ensure exports are intentional and typed.
3. Verify no app-specific assumptions leaked in.
4. Validate consuming apps typecheck/build against the change.
