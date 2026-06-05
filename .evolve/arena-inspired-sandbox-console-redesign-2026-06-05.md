# Arena-Inspired Sandbox Console Redesign

Date: 2026-06-05
Status: design directive
Owner: next UI implementation agent
Scope: `ui/`, shared UI package boundaries, and Tangle Cloud iframe integration behavior

## Compressed Goal Directive

Build the AI Agent Sandbox UI as a Tangle-native Sandbox Console that borrows Arena's workspace model, density, terminal rhythm, route-native resource navigation, metric strips, tables, and evidence rails. Preserve sandbox/cloud concepts: provisioning, runtime backend, operator capacity, lifecycle, workflows, sessions, ports, SSH, secrets, snapshots, and TEE attestation. Do not copy Hyperliquid colors, trading terms, PnL semantics, market metaphors, or chart theater. The target is a cloud operations terminal for agent sandboxes embedded in Tangle Cloud.

Reference this tracker and the 2026-06-05 Codex tracking message whose BLUF was: "Build the AI Agent Sandbox as a Tangle-native Sandbox Console: copy Arena's full-height terminal workspace, dense tables, route-native resource views, metric strips, and evidence rails; do not copy Hyperliquid colors, trading terms, PnL metaphors, or market-first semantics."

## Why This Exists

The research pass showed that the trading Arena has already solved the product shape that the sandbox UI needs:

- full-height application shell
- compact left navigation
- route-native detail sections instead of local tab state
- one primary work surface per route
- dense metric strips and tables
- right-side evidence or identity rail
- minimal explanatory copy
- terminal-grade dark surfaces with real separation

The sandbox blueprint already has the correct backend nouns and operator actions, but the current UI still reads as a card dashboard plus wizard. The redesign should change the user's mental model from "browse a blueprint demo" to "operate an agent compute fleet."

## Non-Goals

- Do not port trading concepts literally.
- Do not use Hyperliquid green as the dominant palette.
- Do not introduce fake market charts for sandbox data.
- Do not replace the backend contract, job IDs, operator API, or metadata schema.
- Do not duplicate non-trivial shared implementations already suitable for `@tangle-network/blueprint-ui` or `@tangle-network/agent-ui`.
- Do not create a marketing landing page.

## Source Evidence

### Arena Sources

- `../ai-trading-blueprint/.evolve/arena-workspace-redesign-2026-06-01.md`
- `../ai-trading-blueprint/.evolve/theme-trace-density-final-20260605/*.png`
- `../ai-trading-blueprint/arena/src/components/layout/ArenaAppShell.tsx`
- `../ai-trading-blueprint/arena/src/components/bot-detail/AgentWorkspaceShell.tsx`
- `../ai-trading-blueprint/arena/src/styles/variables.scss`
- `../ai-trading-blueprint/arena/src/styles/global.scss`

Observed Arena facts:

- `ArenaAppShell` uses a full-height shell with primary nav: Home, Agents, Activity, Observatory, Operators, My Agents, New Agent.
- `AgentWorkspaceShell` uses route-native sections: `performance`, `portfolio`, `runs`, `chat`, `operations`.
- The redesign tracker explicitly says Arena should be "a sidebar-driven trading workspace" with global nav, persistent selected-agent context, one primary work surface, and no body scrolling for core desktop workflows.
- Arena terminal tokens define a dark surface stack, terminal borders, mono data presentation, Tangle purple brand accents, and status/accent colors separated from raw brand color.

### Sandbox Sources

- `README.md`
- `DESIGN.md`
- `metadata/blueprint-metadata.json`
- `ui/src/root.tsx`
- `ui/src/components/layout/Header.tsx`
- `ui/src/routes/_index.tsx`
- `ui/src/routes/create.tsx`
- `ui/src/routes/sandboxes._index.tsx`
- `ui/src/routes/sandboxes.$id.tsx`
- `ui/src/routes/instances._index.tsx`
- `ui/src/routes/instances.$id.tsx`
- `ui/src/routes/workflows._index.tsx`
- `ui/src/routes/workflows.$scope.$workflowId.tsx`
- `ui/src/routes/workflows.create.tsx`
- `packages/agent-ui/src/**`

Observed Sandbox facts:

- Current shell uses top header navigation: Dashboard, Sandboxes, Instances, Workflows, Create.
- Current create route is a 3-step wizard: blueprint, configure, deploy.
- Current sandbox detail uses local tab state: overview, terminal, chat, SSH, secrets, attestation.
- Backend supports three modes: Sandbox, Instance, TEE Instance.
- Runtime backend selection is encoded in `metadata_json.runtime_backend`: Docker, Firecracker, or TEE.
- Operator API supports ports, exec, prompt, task, stop, resume, snapshot, SSH, secrets, and port proxy.
- Lifecycle model includes hot, warm, cold, and gone states.
- TEE flow includes attestation, TEE public key, sealed secrets, and off-chain encrypted secret injection.

### Cloud Sources

- `../dapp/apps/tangle-cloud/src/components/chrome/PageHeader.tsx`
- `../dapp/apps/tangle-cloud/src/components/chrome/MetricStrip.tsx`
- `../dapp/apps/tangle-cloud/src/components/tangleCloudTable/TangleCloudTable.tsx`
- `../dapp/apps/tangle-cloud/src/components/TangleCloudCard.tsx`
- `../dapp/apps/tangle-cloud/src/components/blueprintApps/BlueprintHostCard.tsx`
- `../dapp/apps/tangle-cloud/src/components/blueprintApps/IframeBlueprintLayout.tsx`

Cloud lessons:

- For embedded blueprint apps, the iframe app should own the actual product surface.
- Cloud chrome should stay peripheral.
- Use compact page chrome only when a route needs a header.
- Tables and metric strips should be dense and operational, not decorative.

## Core Product Translation

| Trading Arena Concept | Sandbox Console Concept | Fit | Decision |
|---|---|---:|---|
| Bot | Sandbox or dedicated agent instance | 5/5 | Direct rename. |
| Agent leaderboard | Sandbox explorer | 5/5 | Use full-width sortable/filterable table. |
| Performance chart | Runtime timeline | 4/5 | Show status, CPU, memory, logs, tasks, workflow markers. |
| Fills tape | Execution tape | 5/5 | Exec/prompt/task/workflow/snapshot/lifecycle events. |
| Portfolio | Resource inventory | 4/5 | Processes, ports, sessions, storage, snapshots, cost, allocation. |
| Positions | Active runtime allocations | 4/5 | CPU, memory, disk, ports, workflows, sessions. |
| Runs | Workflow and task replay | 5/5 | Left run list, center transcript/logs, right evidence rail. |
| Chat | Agent sessions | 5/5 | Promote current chat/session UI into first-class workspace route. |
| Operations | Control plane | 5/5 | Lifecycle, network, secrets, auth, attestation, snapshots. |
| Strategy mandate | Sandbox launch spec | 4/5 | Prompt/template, runtime backend, capabilities, ports, secrets path. |
| Operator route | Capacity route | 5/5 | Operator health, backend support, active load, TEE support. |
| Risk readiness | Security/readiness gate | 5/5 | Wallet, service validation, operator capacity, TEE/secrets readiness. |
| Vault/envelope | Secret and attestation envelope | 4/5 | Sealed secrets, public key, attestation, on-chain verification. |
| Market observatory | Fleet observability | 4/5 | Operators, capacity, runtime health, global traces. |

## Target Route Model

### Global Routes

| Route | Name | Primary Job |
|---|---|---|
| `/` | Fleet Console | Operate the sandbox fleet at a glance. |
| `/sandboxes` | Sandbox Explorer | Find, compare, and open cloud-mode sandboxes. |
| `/instances` | Dedicated Instances | Find, compare, and open instance/TEE services. |
| `/workflows` | Automation | See workflow registry and recent executions across resources. |
| `/activity` | Execution Tape | Search all exec, prompt, task, workflow, lifecycle, snapshot, auth, and secret events. |
| `/operators` | Capacity Directory | Inspect operator readiness, backend support, version, load, and TEE support. |
| `/create` | Launch Console | Compile and deploy a new sandbox, instance, or TEE instance. |

### Sandbox Workspace Routes

| Route | Name | Replaces Current |
|---|---|---|
| `/sandboxes/:id/runtime` | Runtime | overview + terminal default route |
| `/sandboxes/:id/sessions` | Sessions | chat tab |
| `/sandboxes/:id/automation` | Automation | workflow links + workflow detail fragments |
| `/sandboxes/:id/network` | Network | ports + SSH tab |
| `/sandboxes/:id/security` | Security | secrets + attestation tabs |
| `/sandboxes/:id/storage` | Storage | snapshot modal + lifecycle state |

### Instance Workspace Routes

Mirror the sandbox workspace model:

- `/instances/:id/runtime`
- `/instances/:id/sessions`
- `/instances/:id/automation`
- `/instances/:id/network`
- `/instances/:id/security`
- `/instances/:id/storage`

If implementation cost is high, build one generic `ResourceWorkspaceShell` that accepts `scope: "sandbox" | "instance"` and resource-specific API adapters.

## Page Design Directives

### 1. Fleet Console

Goal: make the home route feel like an operations terminal, not a dashboard.

Layout:

- full-height app shell
- top compact metric strip
- central fleet runtime timeline or capacity matrix
- right execution tape
- bottom dense table of active resources

Metrics:

- active sandboxes
- running instances
- workflow executions last 24h
- operator capacity available
- failed operations last 24h
- TEE-ready operators

Table columns:

- name
- scope
- status
- backend
- operator
- uptime
- sessions
- workflows
- CPU/memory allocation
- ports
- security posture
- last event

Alternatives ranked:

1. Fleet console with timeline, tape, and active-resource table. Best because it creates an operations cockpit immediately.
2. Operator health map plus resource table. Strong for fleet supply, weaker for user-owned work.
3. Current stat cards and recent cards. Acceptable for a demo, weak for repeated operations.

### 2. Sandbox Explorer

Goal: replace card browsing with a table-first resource explorer.

Layout:

- full-width table
- filter tray for status, backend, operator, TEE, owner, capability, stale state
- optional selected-resource dossier on wide screens
- no repeated card rows

Columns:

- resource name
- sandbox ID
- status: running/stopped/warm/cold/gone/error
- backend: docker/firecracker/TEE
- agent template
- operator
- created
- last event
- active sessions
- workflow count
- exposed ports
- snapshot state
- secrets state
- attestation state

Alternatives ranked:

1. Dense table plus selected dossier. Best for scanning many resources.
2. Table plus right drawer. Good when dossier is too wide.
3. Card list with filters. Current-ish, poorer density.

### 3. Launch Console

Goal: replace wizard feel with a compiler for sandbox intent.

Layout:

- left/middle launch spec composer
- mode segmented control: Sandbox, Instance, TEE Instance
- runtime backend segmented control: Docker, Firecracker, TEE when valid
- capability stack: computer use, all-harness, sidecar capabilities
- ports editor
- env/secrets plan
- operator capacity/readiness rail
- deploy action in the readiness rail

Must preserve:

- existing `useCreateDeploy`
- `metadata_json.runtime_backend`
- Firecracker forcing `tee_required=false`
- TEE forcing `tee_required=true`
- `all_harness` explicit option while keeping ABI field internal
- service validation and operator lookup

Alternatives ranked:

1. Launch console. Best because it shows deployability and resource shape before submission.
2. Resource composer with plan preview. Good, but less decisive.
3. Current three-step wizard. Safe, but too generic and slow.

### 4. Runtime Workspace

Goal: make an opened sandbox feel like a live machine.

Layout:

- compact selected-resource rail or header strip
- center terminal/timeline work surface
- right identity/evidence rail
- bottom or split execution ledger when space allows

Primary modules:

- runtime timeline with status transitions, CPU/memory/disk, task markers, workflow markers, error markers
- terminal using existing operator-backed terminal
- execution ledger
- lifecycle controls: stop, resume, snapshot, delete, create workflow
- connection status and circuit breaker state

Alternatives ranked:

1. Runtime timeline + terminal + event ledger. Best because it joins state, action, and evidence.
2. Terminal-first with metric rail. Good for power users, weaker for fleet health.
3. Current overview cards plus terminal tab. Fragmented and slower.

### 5. Sessions Workspace

Goal: make agent chat/session work a real product surface.

Layout:

- left session browser from `packages/agent-ui`
- center transcript
- right run/evidence rail for the selected session
- compact composer

Do:

- preserve owner+scope isolation
- preserve prompt/task payload contract differences
- show session ID and scope when useful

Do not:

- bury chat behind disabled tabs without a route
- show giant empty cards

### 6. Automation Workspace

Goal: make workflows feel like scheduled operations, not JSON forms.

Layout:

- workflow registry table
- execution replay view
- schedule/status rail
- action controls: trigger, cancel, edit, create

Map Arena Runs:

- left: workflow/run history
- center: transcript/log/output
- right: trigger, schedule, target, operator, tx/evidence

Alternatives ranked:

1. Runs replay shell. Best because it turns automation into audit evidence.
2. Registry table plus detail drawer. Good first slice.
3. Current card/detail JSON panels. Too generic.

### 7. Network Workspace

Goal: make ports, proxy, and SSH operationally obvious.

Modules:

- exposed ports table
- proxy URLs
- SSH key list
- connection command
- add/revoke key forms
- runtime backend network notes
- Firecracker DNAT state when available

Alternatives ranked:

1. Network table plus command rail. Best for copy/run workflows.
2. Split ports and SSH panels. Acceptable.
3. SSH tab only. Too narrow.

### 8. Security Workspace

Goal: unify secrets, sealed secrets, TEE attestation, auth, and trust state.

Modules:

- secrets editor
- wipe secrets action
- TEE public key
- sealed secret injection
- attestation report
- session auth state
- on-chain service/resource verification

Alternatives ranked:

1. Security control plane. Best because trust state is one mental model.
2. Separate Secrets and Attestation routes. Acceptable if implementation needs smaller slices.
3. Current tabs. Hides the relationship between secrets and attestation.

### 9. Storage Workspace

Goal: expose lifecycle and persistence as first-class cloud concepts.

Modules:

- snapshot ledger
- hot/warm/cold/gone lifecycle state
- auto-commit policy
- S3/BYOS3 destination
- restore/resume path
- retention timers

Alternatives ranked:

1. Snapshot ledger plus lifecycle rail. Best for operational correctness.
2. Storage drawer in runtime. Good smaller slice.
3. Snapshot modal only. Too invisible for a sandbox product.

### 10. Operators / Capacity

Goal: show whether the fleet can run the requested workload.

Columns:

- operator
- endpoint status
- active sandboxes
- available capacity
- backend support
- TEE backend support
- version
- error rate
- last heartbeat
- service ID

Alternatives ranked:

1. Capacity directory table. Best for cloud-native operations.
2. Operator cards plus filters. Lower density.
3. Hide operators inside deploy flow. Insufficient for trust.

## Visual System Directive

Use Arena's structural visual grammar, not Arena's literal market skin.

Adopt:

- full-height dark console
- high-contrast surface stack
- thin but visible borders
- mono numerals and IDs
- compact metric labels
- dense tables
- restrained purple brand accents
- teal/green only as success/ready, not dominant brand
- amber for waiting/degraded
- red for destructive/error
- direct component labels and terse row metadata

Avoid:

- giant cards
- nested cards
- hero copy
- route/status narration that repeats controls
- decorative gradients or orbs
- huge default empty states
- disabled-looking "ready" panels with no live evidence

Token direction:

- keep Tangle dark neutral base from the sandbox/Cloud UI
- add terminal-specific tokens similar in purpose to Arena's `--arena-terminal-*`
- name them sandbox-native, for example `--sandbox-console-bg`, `--sandbox-console-surface`, `--sandbox-console-panel`, `--sandbox-console-border`, `--sandbox-console-accent`, `--sandbox-console-brand`
- keep status tokens separate from brand tokens

## Component Inventory

Likely new or refactored components:

- `SandboxConsoleShell`
- `ConsoleNavRail`
- `ConsoleAccountDock`
- `ResourceWorkspaceShell`
- `ResourceContextStrip`
- `ResourceMetricStrip`
- `ResourceEvidenceRail`
- `ExecutionTape`
- `RuntimeTimeline`
- `ResourceExplorerTable`
- `LaunchConsole`
- `ReadinessRail`
- `CapabilityStack`
- `NetworkPanel`
- `SecurityPanel`
- `StorageLedger`
- `WorkflowReplay`
- `OperatorCapacityTable`

Promote shared code when it crosses product boundaries:

- chain/wallet/infra primitives: `@tangle-network/blueprint-ui`
- session/chat/terminal primitives: `@tangle-network/agent-ui`
- sandbox-specific shell, runtime copy, route composition: local `ui/`

## Data and API Mapping

Use existing data before adding APIs.

Existing useful contracts:

- on-chain jobs `0..4`
- internal workflow tick `255`
- `metadata_json.runtime_backend`
- `capabilities_json`
- operator API ports
- operator API exec
- operator API prompt
- operator API task
- operator API stop/resume
- operator API snapshot
- operator API SSH
- operator API secrets
- TEE attestation and sealed secrets APIs
- runtime metrics fields in `DESIGN.md`
- sandbox lifecycle states

Possible later API gaps:

- unified event stream for Execution Tape
- resource metric time series for Runtime Timeline
- operator capacity history
- snapshot ledger pagination
- workflow execution history pagination

If an API gap exists in the first implementation slice, render the component from currently indexed events and mark the data source explicitly in code. Do not fake precision.

## Implementation Order

1. Add console shell and left navigation without changing backend behavior.
2. Convert global home to Fleet Console.
3. Convert sandbox and instance lists to dense explorers.
4. Introduce route-native workspace shell for sandbox detail.
5. Move current terminal/chat/SSH/secrets/attestation views into resource routes.
6. Rebuild create as Launch Console while preserving deploy hook contracts.
7. Add Execution Tape and Workflow Replay from available data.
8. Add Runtime Timeline from real metrics/events only.
9. Consolidate visual tokens and shared primitives.
10. Run UI tests, typecheck, and screenshot verification.

## Acceptance Criteria

- Desktop core routes use a full-height shell with no body scroll for the main workflow.
- Global nav is compact and persistent.
- Opened resources use route-native sections, not local tab-only state.
- Home reads as fleet operations, not marketing dashboard.
- Create reads as launch compiler, not generic ABI wizard.
- Lists are table-first.
- Runtime workspace surfaces terminal, state, lifecycle actions, and evidence together.
- Security makes secrets and attestation one coherent trust model.
- Storage exposes snapshot and hot/warm/cold lifecycle state.
- Tangle theme is recognizable without copying Hyperliquid green.
- No fake charts or unsupported metrics.
- Existing tests still pass.

## Verification Gate

Minimum for UI-only slices:

- `pnpm --dir ui test`
- `pnpm --dir ui typecheck`
- desktop screenshot at 1440x900 for `/`, `/sandboxes`, `/create`, and one resource workspace route
- mobile screenshot for global nav and create route

For any runtime/operator API change, also run the relevant Rust tests from `CLAUDE.md`.

## Coordination Notes

The initial research pass was read-only. At the time of tracking, the sandbox repo was clean on `fix/dark-card-depth`. Other related repos had active edits by other agents:

- `../ai-trading-blueprint`
- `../dapp`
- `../tnt-core`

Do not overwrite those branches. Treat Arena and Cloud as reference sources unless the user explicitly asks for cross-repo edits.
