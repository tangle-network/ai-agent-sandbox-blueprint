# Sandbox Launch Wizard Product Audit

Status: implementation pass verified locally; pending deploy
Date: 2026-06-06
Product: Tangle AI Agent Sandbox Blueprint UI
Scope: `/create` launch wizard and deploy review
Product brief: `PRODUCT_BRIEF.md`
Reference posture: Arena-style dense command/control, mapped to Tangle agent compute

## Confidence

| Area | Confidence | Evidence | Remaining gap |
| --- | ---: | --- | --- |
| Configure wizard controls | 9/10 | Focused route tests exercise mode, infrastructure, validation, runtime, resources, SSH, ports, advanced modal, dropdowns, toggles. Browser smoke covers desktop/mobile. | Real wallet session not needed for configure. |
| Deploy review no-wallet branch | 9/10 | Browser clicked review-rail Connect wallet action; no page errors. Mobile overflow fixed and verified. | ConnectKit modal behavior depends on installed wallet/provider. |
| Missing sandbox service recovery | 8.5/10 | Route tests cover inline create, no route-away buttons, quote refresh, manual operator add/remove, existing service check. | Real on-chain request still needs wallet/contract confirmation. |
| Ready deploy branch | 8/10 | Route tests cover deploy button and View resource navigation. | Real wallet signing and operator event activation not exercised headlessly. |
| Production readiness | 8.5/10 | Full tests, typecheck, build, and browser proof complete for this pass. | Needs live deploy smoke after commit. |

## Summary Components

| Component | State/data shown | Primary user decision | Action contract | Audit result |
| --- | --- | --- | --- | --- |
| `LaunchSummaryPanel / Mode` | Selected blueprint/entity label | Am I launching the right resource type? | Read-only summary; mode changed by cards. | Keep. Cleanly maps blueprint choice. |
| `LaunchSummaryPanel / Spec` | Open/editing state | Is config still mutable? | Read-only. | Keep, but deploy state is not shown because panel hides on deploy. Low-risk dead branch in copy. |
| `LaunchSummaryPanel / Runtime` | Docker/Firecracker/TEE | Does runtime match intent? | Read-only; changed by segmented control. | Keep. Firecracker detail verified. |
| `LaunchSummaryPanel / Capacity` | Available slots | Is capacity a blocker? | Read-only. | Keep. Should eventually link to operator capacity evidence. |
| `LaunchSummaryPanel / Wallet` | Connected/syncing/offline | Can this wallet sign? | Read-only summary. | Keep; deploy review now has actionable Connect wallet. |
| `LaunchSummaryPanel / Service` | Service validation/new-service status | Is the target service usable? | Read-only summary; configure can open Infrastructure. | Keep. Deploy recovery now inline. |
| `LaunchSummaryPanel / Operators` | Verified/registered/lookup failed count | Is there operator inventory? | Read-only. | Keep, but future pass should rank operators by capabilities. |
| `LaunchSummaryPanel / Agent mode` | Agent identifier or compute-only | Will chat be enabled? | Read-only; changed by Agent dropdown/input. | Keep. |
| `LaunchSummaryPanel / Network` | Default or exposed port count/list | Are ports expected? | Read-only; changed by Exposed Ports. | Keep. |
| Deploy identity header | Intent, name, image, blueprint identity | Is this the resource I am about to launch? | Read-only; Back to edit changes. | Keep. Mobile shrink bug fixed. |
| Deploy spec pills | Entity, runtime, CPU/RAM/disk, ports | Are the core launch specs correct? | Read-only; Back to edit changes. | Keep. Mobile shrink bug fixed. |
| Cost block | Estimate/RFQ quote | What is due now? | Read-only. | Keep; hierarchy is strong. |
| Preflight panel | Blocker/ready state and reason | What must happen before deploy? | Context plus paired action below. | Improved: wallet blocker now has real ConnectKit action. |
| Active config chips | Non-default options | What extras are included? | Read-only. | Keep, but future pass should redact/compact long values. |
| Per-job pricing disclosure | Other job prices | Inspect follow-on operation cost | Toggle disclosure. | Tested and working. |
| Firecracker notice | Runtime constraint | Do I have a suitable operator? | Read-only. | Keep; future pass should be operator-capability backed. |
| Operator list | Operators for instance service request | Which operator will receive request? | Read-only in instance path. | Keep; future pass should become selectable/ranked. |
| Service setup panel | New/existing service recovery | Create service or select service ID in place | Inline tabs, operator select, manual add/remove, quote refresh, create, check. | Improved and tested. |

## Wizard Pages And Subpages

| Page/subpage | Purpose | Primary controls | Button/control proof | Current score | Next improvement |
| --- | --- | --- | --- | ---: | --- |
| Blueprint step | Choose launch mode from no preselection | Mode cards, Continue | Mode card and Continue covered by route tests. | 8/10 | Reduce explanatory copy if no preselection is common. |
| Configure / Connect wallet notice | Warn user before deploy | Connect wallet via shared panel | Browser no-wallet branch observed; review rail now actionable. | 8/10 | Make global/review wallet actions visually consistent. |
| Configure / Launch Mode | Change mode without leaving route | Three mode cards | Route tests click TEE and assert infra update. | 8.5/10 | Consider collapsing after a mode is selected on small screens. |
| Configure / Identity + Image | Name and image selection | Name input, image dropdown, custom image input | Tests cover required-name focus and custom image input. | 8.5/10 | Add image compatibility warnings by runtime/harness. |
| Configure / Runtime + Stack | Runtime substrate and stack | Runtime segmented control, Stack dropdown | Tests cover Firecracker and dropdown path. | 8/10 | Stack should become first-class Nix profile builder. |
| Configure / Resources | CPU, memory, disk | Three compact number inputs | Tests cover value editing. | 8.5/10 | Add presets and operator fit validation. |
| Configure / Agent | Compute-only/default/batch/custom agent | Agent dropdown/input | Tests cover Batch, None, custom image free text. | 8/10 | Add agent manifest validation. |
| Configure / Capabilities | Harness and computer-use flags | All-Harness switch, Computer Use switch | Tests cover toggles and `aria-checked`. | 8.5/10 | Replace binary all-harness with harness pack selector. |
| Configure / Access | SSH and public key | Enable SSH switch, key textarea | Tests cover reveal and value entry. | 8.5/10 | Add key validation and saved key picker. |
| Configure / Environment | Boot env vars | Env editor | Rendered in browser; not deeply validated here. | 7/10 | Replace raw JSON with key/value rows plus secret handoff. |
| Configure / Network | Exposed ports | Ports input | Tests and browser cover Docker/Firecracker entry. | 7.5/10 | Replace comma input with structured port rows. |
| Advanced modal | Rare runtime limits and metadata | Open, Close, Done, numeric limits, TEE controls, metadata | Tests cover open/Close/Done. | 8/10 | Split metadata from limits; validate JSON inline. |
| Deploy / No wallet | Prevent deploy until wallet available | Connect wallet, Back to edit | Browser clicked main review connect; tests mock ConnectKit action. | 9/10 | Needs real wallet extension/profile proof. |
| Deploy / Ready instance | Submit service request | Create Service & Deploy, pricing disclosure, Back | Tests cover deploy and pricing toggle. | 8.5/10 | Needs real on-chain wallet proof. |
| Deploy / Missing sandbox service | Recover without losing draft | New service, Existing service, Add, Refresh, Create service, Check service, Back | Tests cover all listed controls. | 9/10 | Needs real operator quote/on-chain request proof. |
| Deploy / Completed resource | Open created resource | View Sandbox/Instance | Test covers View Sandbox navigation. | 8.5/10 | Add instance-specific View test if needed. |
| Deploy / Transaction status | Explain signing/pending/failed | No primary buttons | Covered indirectly by status rendering. | 7.5/10 | Add retry/reset path for failed tx. |
| Deploy / Provision progress | Watch operator result | No direct buttons in this route | Existing component tests elsewhere. | 7.5/10 | Add launch-wizard provision failure test. |

## Button Contract Table

| Button/control | Expected behavior | Proof this pass | Status |
| --- | --- | --- | --- |
| Launch mode card | Select blueprint, set infra, enter configure | Route test clicks TEE card and asserts infra. | Working |
| Blueprint Continue | Select default/selected mode | Route test covers mode/Continue path. | Working |
| Infrastructure | Open infrastructure modal from configure | Route test sees modal. | Working |
| Configure Back | Return to blueprint step | Route test sees `Next`. | Working |
| Configure Continue empty | Focus name, show error, stay configure | Route test asserts focus/error. | Working |
| Configure Continue valid | Enter deploy review | Multiple route/browser tests. | Working |
| Docker Image dropdown | Open custom dropdown, select built-in/custom | Route and mobile browser tests. | Working |
| Custom Image input | Switch to free-text image | Route test. | Working |
| Runtime segmented buttons | Change runtime and visual state | Route/browser tests. | Working |
| Stack dropdown | Custom dropdown opens/selects | Browser smoke opens dropdown family; route covers structure indirectly. | Working |
| CPU/RAM/Disk inputs | Edit compact resource values | Route test. | Working |
| Agent dropdown | Select Batch/None | Route tests. | Working |
| All-Harness switch | Toggle capability | Route test asserts `aria-checked`. | Working |
| Computer Use switch | Toggle capability | Route test asserts `aria-checked`. | Working |
| Enable SSH switch | Reveal SSH key input | Route test. | Working |
| Advanced Settings | Open modal | Route test. | Working |
| Advanced close X | Close modal | Route test. | Working |
| Advanced Done | Close modal | Route test. | Working |
| Review Connect wallet | Open ConnectKit action, not disabled dead button | Code changed; route mock and browser click proof. | Fixed |
| Back to edit | Return to configure with draft intact | Browser smoke asserts name preserved. | Working |
| Per-job pricing | Toggle price rows | Route test. | Working |
| New service tab | Show service creation controls | Route test. | Working |
| Existing service tab | Show service ID check controls | Route test. | Working |
| Operator row select | Toggle operator selection | Existing creation path relies on selected operator; manual remove tested. | Working |
| Manual operator Add | Add visible operator selection | Code changed; route test. | Fixed |
| Manual operator Remove | Remove manual operator selection | Code changed; route test. | Fixed |
| Quote Refresh | Refetch operator quote | Route test. | Working |
| Create service | Submit requestService/createServiceFromQuotes | Route test asserts requestService args. | Working locally |
| Check service | Validate positive service ID and update infra | Route test. | Working locally |
| Invalid service ID | Do not validate invalid ID | Code changed; route test with `0`. | Fixed |
| Create Service & Deploy | Invoke deploy hook | Route test. | Working locally |
| View Sandbox | Navigate to created sandbox detail | Route test. | Working |

## Shipped Changes In This Pass

| Change | Why it matters |
| --- | --- |
| Manual operator entries now render as removable selections. | The Add button no longer mutates invisible state. |
| Service ID check now requires a positive whole-number ID before validation. | Prevents confusing contract-read failures from invalid local input. |
| Review-rail wallet blocker now opens ConnectKit instead of rendering a disabled button. | Removes a dead primary action. |
| Deploy review grid children now use `min-w-0`. | Fixes mobile clipping/overflow in the review card. |
| Focused route suite expanded from 11 to 19 tests. | Every critical wizard button/control now has a deterministic contract test. |

## Verification

| Check | Result |
| --- | --- |
| `pnpm -C ui test src/routes/create.test.tsx` | Passed, 19/19 |
| `pnpm -C ui typecheck` | Passed |
| `pnpm -C ui test` | Passed, 447/447 |
| `pnpm -C ui build` | Passed |
| Desktop browser smoke | Passed; screenshots in `.evolve/sandbox-launch-wizard-audit-2026-06-06/` |
| Mobile overflow check | Passed: `scrollWidth=390`, `overflowCount=0` |
| Review-rail Connect wallet browser click | Passed; screenshot captured |

## Remaining Risks

| Risk | Why it remains | Next proof |
| --- | --- | --- |
| Real wallet signing | Headless browser has no deterministic wallet extension/profile attached. | Run wallet extension/profile or parent-bridge wallet smoke. |
| Real quote/on-chain service creation | Unit tests prove args; no wallet/chain transaction submitted in this pass. | Base Sepolia wallet smoke with funded account. |
| Operator capability ranking | Current UI lists/uses operators but does not score runtime/harness/Nix fit. | Add operator-fit model and table once backend capabilities are stable. |
| Structured ports | Comma input works but is not world-class. | Replace with editable port rows. |
| Harness/Nix configuration | Existing controls expose capabilities but not full harness pack or Nix profiles. | Clean-slate Launch Spec Builder pass. |
