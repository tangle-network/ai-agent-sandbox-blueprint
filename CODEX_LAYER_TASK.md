Create architecture contracts for layered blueprint ecosystem.

Repo: ~/code/ai-agent-sandbox-blueprint
Branch: feat/layered-architecture-contracts

Deliverables:
1) docs/ARCHITECTURE.md with:
   - Repo roles and boundaries:
     * microvm-blueprint (infra only)
     * sandbox-runtime (generic sandbox runtime contracts + adapters)
     * ai-agent-sandbox-blueprint (product blueprint)
     * ai-trading-blueprints (product blueprint)
     * openclaw-hosting-blueprint (product blueprint)
   - Explicit dependency direction rules (who can import what)
   - Why microvm exists separately even if sandbox runtime has microvm adapter
   - Migration phases (now, next, deprecation)
2) docs/CONTRACTS.md defining minimal interfaces:
   - SandboxProvider
   - RuntimeAdapter
   - TemplatePack
   - TenantProfile
   with concise Rust-like trait sketches and data contract fields.
3) Update README with a short "Layered Architecture" section linking docs.

Constraints:
- No fluff. Clear and actionable.
- Keep docs opinionated and implementation-ready.
- Commit changes.

When done: openclaw system event --text "Done: layered architecture contracts docs committed" --mode now
