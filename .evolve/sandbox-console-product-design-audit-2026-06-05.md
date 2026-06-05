# Sandbox Console Product Design Audit

Date: 2026-06-05
Status: implementation trace
Live target: https://agent-sandbox.blueprint.tangle.tools

## Compressed Goal Directive

Ship a 10/10 Tangle-native Agent Sandbox Console by rebuilding `/create` from a duplicated wizard into a compact launch composer, fixing the local console control system, and defaulting production to Base Sepolia. Reference `PRODUCT_BRIEF.md` and this tracking file. Preserve Arena's density, workspace rhythm, and high-rigor control states, but keep sandbox/cloud concepts and Tangle branding.

## Complaint Ledger

| Defect | Decision |
|---|---|
| Black borders on buttons, inputs, forms, components | Replace launch controls with local console primitives using soft borders, solid surfaces, hover/focus states, and theme tokens. |
| Transparent/drop-offscreen dropdowns | Replace shared sidebar chain switcher with a solid local popover that opens up in the desktop rail and down in mobile. Replace create selects with solid in-page controls. |
| Checkboxes feel generic | Use a console toggle/check control with icon state, hover surface, and distinct selected treatment. |
| CPU/memory/disk take a whole row each | Render resources as compact inline numeric controls. |
| Enable SSH does not work | Bind SSH enablement directly to form state and reveal the public-key textarea when enabled. |
| Text inputs flush against edge | Use padded compact input primitives with consistent font size. |
| Environment variables use smaller font | Normalize env editor row text to the same console text scale. |
| Advanced Options unclear | Move metadata, lifetime, idle timeout, and TEE-specific settings into an Advanced modal. |
| Launch modes, catalog, readiness, runtime capacity duplicated | Keep one mode strip and one readiness/deploy summary. Remove compiler phases and duplicate catalog/readiness chrome. |
| Continue appears broken | Add attempted-submit validation, focus the missing name, and show a visible error. |
| Deploy and Back buttons weak | Use launch-specific primary/secondary action styles. |
| Base Sepolia expected, Tangle Local selected | Register Base Sepolia first, hide Local unless explicitly enabled, and derive wallet chains from configured networks. |

## Alternatives Ranked

| Area | Best | Second | Third |
|---|---|---|---|
| Launch information architecture | One-page composer + deploy review | Wizard with persistent summary | Current rail/catalog/readiness split |
| Advanced settings | Modal by option family | Collapsible drawer | Inline collapsed shared form sections |
| Resource sizing | Inline compact steppers/inputs | Preset cards plus custom row | Three full-width number fields |
| Chain selector | Local solid popover with collision-aware placement | Radix select wrapper | Shared transparent dropdown |
| Readiness | Decision summary near deploy action | Metric strip plus right rail | Decorative multi-row launch readiness panel |

## Implementation Scope

- `ui/src/routes/create.tsx`
- `ui/src/components/shared/EnvEditor.tsx`
- `ui/src/components/console/ConsoleChainSwitcher.tsx`
- `ui/src/components/console/ConsoleShell.tsx`
- `ui/src/lib/contracts/chains.ts`
- `ui/src/providers/Web3Provider.tsx`
- `ui/env.d.ts`
- `ui/.env.example`
- focused route tests and build/typecheck verification

## Verification Notes

- `pnpm --dir ui test create.test.tsx`
- `pnpm --dir ui typecheck`
- `pnpm --dir ui build`
- Browser proof on `http://localhost:1339/create?blueprint=ai-agent-instance-blueprint`:
  - compact wallet prompt renders instead of the large empty-state card
  - CPU/RAM/disk values fit without clipping
  - Enable SSH toggles on and reveals the SSH public-key textarea
  - Advanced Settings opens as a modal
  - Base Sepolia chain menu opens upward in the sidebar and stays on screen
  - empty Continue shows `Instance name is required` and focuses `Instance Name`
  - deploy summary reports `blueprint 11 / service 2` for the instance mode when Local is not explicitly enabled
