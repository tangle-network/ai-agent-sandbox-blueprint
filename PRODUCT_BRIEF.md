# AI Agent Sandbox Console Product Brief

Date: 2026-06-05
Status: active implementation brief

## Goal

Make the AI Agent Sandbox blueprint feel like a Tangle-native cloud operations console for launching and operating agent compute. The UI should borrow Arena's density, workspace rhythm, decisive control states, and proof-oriented rails while preserving sandbox concepts: provisioning, runtime backend, operator capacity, service validation, workflows, sessions, ports, SSH, secrets, snapshots, and TEE attestation.

## Current Rating

The live app is visually improved but still reads as 5/10 because the launch path is a generic wizard wrapped in extra chrome. The highest-impact defects are black-bordered controls, transparent/drop-offscreen dropdowns, weak buttons, duplicated launch/catalog/readiness panels, unclear advanced options, no useful validation feedback on Continue, and a production network default that still falls back to Tangle Local.

## Product Standard

- Launch is a compact spec composer, not a blueprint demo wizard.
- One mode selector chooses Sandbox, Dedicated Instance, or TEE Instance. Do not duplicate it as a catalog elsewhere on the same page.
- Runtime, stack, resources, agent mode, ports, SSH, env vars, and advanced settings are all directly editable with compact controls.
- Advanced options belong in a modal when they are not part of the normal launch decision.
- Readiness should summarize wallet, service, capacity, operator, network, and cost only when those facts drive a deploy decision.
- Buttons, selects, inputs, toggles, and checkboxes need hover, focus, disabled, and selected states that match the console theme.
- Production defaults to Base Sepolia; Local is a development option only.

## Design Direction

Use the Arena-inspired shell already started in `.evolve/arena-inspired-sandbox-console-redesign-2026-06-05.md`, but remove remaining demo scaffolding. Favor dense panels, segmented controls, inline resource inputs, solid popovers, small operational copy, and clear state changes. Do not use Hyperliquid colors, trading semantics, fake market widgets, or explanatory route narration.

## Tangle Brand Direction

The product should read as Tangle Compute Fabric: a dark-first, technical cloud console where sandboxes, instances, operators, workflows, ports, secrets, and TEE attestations are visible as a programmable resource fabric. Tangle violet identifies the product family, but runtime state needs its own semantic color system: execution cyan, network blue, TEE amber, storage indigo, and risk coral. Avoid a one-note purple UI, generic SaaS card grids, and blank centered empty states.

## Quality Gate

The `/create` page must work end to end: selecting a mode updates infra, SSH reveals a key field, Continue either focuses the missing name field with a visible error or moves to deploy review, deploy/back controls share the same design language, and the chain selector is readable and stays on screen.
