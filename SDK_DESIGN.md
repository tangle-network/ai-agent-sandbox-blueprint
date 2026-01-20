# Public SDK Surface Design

## Overview

This document defines the public SDK surface for the AI Agent Sandbox, with careful consideration for:
- Security boundaries (what to expose vs keep private)
- Prevention of reverse engineering proprietary logic
- Customer needs vs internal implementation details
- Clean separation between public API and internal packages

## Current Package Audit

### Sensitivity Classification

| Package | Exports | Sensitivity | Customer Need | Recommendation |
|---------|---------|-------------|---------------|----------------|
| `sdk` | 148 | LOW-MEDIUM | HIGH | **PUBLIC** |
| `sdk-core` | 225 | HIGH (auth) | HIGH | **PUBLIC** (careful with auth docs) |
| `sdk-service` | 486 | **VERY HIGH** | MEDIUM | **PARTIAL** - facade only |
| `sdk-telemetry` | 156 | MEDIUM | HIGH | **PUBLIC** |
| `sdk-provider-opencode` | 124 | HIGH | HIGH | **PUBLIC** |
| `sdk-data` | 293 | HIGH | MEDIUM | **PARTIAL** - hide scoring |
| `sdk-transport-*` | varies | LOW | HIGH | **PUBLIC** |
| `sdk-signals` | 54 | MEDIUM | HIGH | **PUBLIC** |
| `sdk-memory` | 245 | MEDIUM | MEDIUM | **PARTIAL** |
| `sdk-batch` | 107 | MEDIUM | LOW-MEDIUM | **PUBLIC** |
| `agent-interface` | minimal | STRATEGIC | CRITICAL | **PUBLIC** (versioned) |

### Security Concerns by Package

#### sdk-service (VERY HIGH risk)

**Exposes business-critical internals:**
- Rate limiting tiers and algorithms (`TIER_LIMITS`)
- Billing calculation logic
- Webhook signature generation
- Session claim schema and TTLs
- Audit log structure

**Recommendation:** Split into public facade + private internals.

```typescript
// PUBLIC (safe to expose)
export { CustomerService } from './customer/service';
export { createBillingRoutes } from './billing/routes';
export { createWebhookHandler } from './webhooks/handler';
export { RateLimiterService } from './ratelimit/service';
export type { Customer, Subscription, UsageSummary };

// PRIVATE (keep internal)
// - TIER_LIMITS constants
// - Webhook HMAC signing implementation
// - Session JWT claim schema
// - Billing calculation formulas
// - Audit event structure
```

#### sdk-core (HIGH risk in auth subsystem)

**Exposes auth implementation:**
- `ProductTokenIssuer` - JWT generation logic
- Token verification internals
- HMAC-SHA256 signing utilities

**Mitigation:** These are standard patterns, but documentation must warn about secret protection. Keep public but add security guidance.

#### sdk-data (HIGH risk in algorithms)

**Exposes proprietary analysis:**
- Recommendation scoring weights
- Failure classification rules
- Signal detection heuristics
- AI classifier prompts

**Recommendation:** Make scoring configurable with defaults private.

```typescript
// PUBLIC
export { QueryEngine } from './query/engine';
export { InsightsEngine } from './insights/engine';
export type { TraceEvent, AnalysisResult };

// PRIVATE (move to internal)
// - DEFAULT_RECOMMENDATION_CONFIG
// - Scoring weight constants
// - Failure classification rules
// - Signal detection patterns
```

## Recommended Architecture

### Option A: Single Package with Selective Exports (Simpler)

Keep existing packages but control what's exported via `package.json` exports map:

```json
{
  "name": "@tangle-network/sdk",
  "exports": {
    ".": "./dist/public/index.js",
    "./internal": "./dist/internal/index.js"
  }
}
```

**Pros:** Less refactoring, single package to maintain
**Cons:** Internal code still ships in bundle, can be inspected

### Option B: Separate Public SDK Package (Recommended)

Create a new `@tangle-network/sdk-public` that re-exports only safe APIs:

```
packages/
├── sdk-public/           # NEW - Public surface only
│   ├── src/
│   │   ├── index.ts      # Curated exports
│   │   ├── client.ts     # UnifiedClient
│   │   ├── types.ts      # Public type definitions
│   │   └── errors.ts     # Public error types
│   └── package.json
├── sdk/                  # Internal - full implementation
├── sdk-core/             # Internal - infrastructure
├── sdk-service/          # Internal - SaaS building blocks
└── ...
```

**Pros:** Clean separation, internal code doesn't ship to customers
**Cons:** More packages to maintain, need to keep in sync

### Option C: Facade Pattern with Code Splitting (Most Secure)

Internal packages compile to private bundle; public SDK imports only the facade:

```
packages/
├── sdk-public/                    # Ships to customers
│   └── src/index.ts
├── @internal/
│   ├── sdk-core-internal/         # Never published
│   ├── sdk-service-internal/      # Never published
│   └── sdk-algorithms/            # Never published
└── sdk-facades/                   # Public facades that wrap internal
    ├── customer-facade.ts
    ├── billing-facade.ts
    └── analytics-facade.ts
```

**Pros:** Maximum security, proprietary code never ships
**Cons:** Significant refactoring, complexity

## Recommended Public SDK Surface

### Core Client API

```typescript
// @tangle-network/sdk-public

// ═══════════════════════════════════════════════════════════════════
// MAIN CLIENTS
// ═══════════════════════════════════════════════════════════════════

export { createClient, UnifiedClient } from './client';
export { SidecarClient } from './sidecar';
export { BlueprintClient } from './blueprint';

// ═══════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════

export interface ClientConfig {
  mode: 'centralized' | 'decentralized';

  // Centralized mode
  orchestratorUrl?: string;
  apiKey?: string;

  // Decentralized mode
  wallet?: WalletClient;
  blueprintAddress?: string;
  rpcUrl?: string;
}

// ═══════════════════════════════════════════════════════════════════
// SANDBOX TYPES
// ═══════════════════════════════════════════════════════════════════

export interface SandboxConfig {
  cpuCores: number;
  memoryMB: number;
  diskGB: number;
  agentBackend?: string;
  agentIdentifier?: string;
  env?: Record<string, string>;
  sshEnabled?: boolean;
  sshPublicKey?: string;
  ttlBlocks?: number;
  idleTimeoutSeconds?: number;
  snapshotDestination?: string;
}

export interface Sandbox {
  id: string;
  endpoint: string;
  sshHost?: string;
  sshPort?: number;
  sshUser?: string;
  streamEndpoint: string;
  expiresAt: Date;
  status: 'running' | 'stopped' | 'expired';
}

export interface SSHCredentials {
  host: string;
  port: number;
  user: string;
}

// ═══════════════════════════════════════════════════════════════════
// EXECUTION TYPES
// ═══════════════════════════════════════════════════════════════════

export interface ExecOptions {
  cwd?: string;
  env?: Record<string, string>;
  timeoutMs?: number;
}

export interface ExecResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

export interface PromptOptions {
  sessionId?: string;
  model?: string;
  context?: Record<string, unknown>;
  timeoutMs?: number;
}

export interface PromptResult {
  success: boolean;
  response: string;
  error?: string;
  traceId: string;
  durationMs: number;
  inputTokens: number;
  outputTokens: number;
  sessionId: string;
}

export interface TaskOptions extends PromptOptions {
  maxTurns?: number;
}

export interface TaskResult extends PromptResult {
  turnsUsed: number;
}

// ═══════════════════════════════════════════════════════════════════
// BATCH TYPES
// ═══════════════════════════════════════════════════════════════════

export interface BatchOptions {
  operators?: string[];
  distribution?: 'round_robin' | 'cheapest' | 'random';
}

export interface Batch {
  id: string;
  sandboxIds: string[];
  endpoints: string[];
}

export interface BatchTaskOptions {
  parallel?: boolean;
  aggregation?: 'all' | 'first_success' | 'majority';
}

export interface BatchResult {
  batchId: string;
  results: TaskResult[];
  succeeded: number;
  failed: number;
}

// ═══════════════════════════════════════════════════════════════════
// WORKFLOW TYPES
// ═══════════════════════════════════════════════════════════════════

export interface WorkflowDefinition {
  name: string;
  tasks: WorkflowTask[];
  trigger: WorkflowTrigger;
  sandboxConfig: SandboxConfig;
}

export interface WorkflowTask {
  id: string;
  prompt: string;
  dependsOn?: string[];
}

export interface WorkflowTrigger {
  type: 'manual' | 'cron' | 'webhook' | 'event';
  config?: string; // Cron expression or webhook config
}

export interface Workflow {
  id: string;
  name: string;
  status: 'active' | 'paused';
}

export interface WorkflowRun {
  id: string;
  workflowId: string;
  status: 'running' | 'completed' | 'failed';
  startedAt: Date;
  completedAt?: Date;
}

// ═══════════════════════════════════════════════════════════════════
// OPERATOR TYPES (decentralized mode)
// ═══════════════════════════════════════════════════════════════════

export interface Operator {
  address: string;
  endpoint: string;
  pricing: OperatorPricing;
  reputation: number;
  capacity: {
    cpu: number;
    memoryMB: number;
    available: boolean;
  };
}

export interface OperatorPricing {
  pricePerHourWei: bigint;
  pricePerPromptWei: bigint;
  acceptedAssets: string[];
}

export interface OperatorFilter {
  minReputation?: number;
  maxPricePerHour?: bigint;
  requiredCapacity?: {
    cpu?: number;
    memoryMB?: number;
  };
}

// ═══════════════════════════════════════════════════════════════════
// STREAMING
// ═══════════════════════════════════════════════════════════════════

export interface AgentEvent {
  type: 'message' | 'tool_call' | 'tool_result' | 'thinking' | 'error' | 'done';
  data: unknown;
  timestamp: number;
}

// ═══════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════

export class SandboxError extends Error {
  code: string;
  statusCode?: number;
}

export class AuthenticationError extends SandboxError {}
export class QuotaExceededError extends SandboxError {}
export class PaymentRequiredError extends SandboxError {}
export class NotFoundError extends SandboxError {}
export class TimeoutError extends SandboxError {}
```

### UnifiedClient Interface

```typescript
export interface UnifiedClient {
  // ═══════════════════════════════════════════════════════════════
  // SANDBOX LIFECYCLE
  // ═══════════════════════════════════════════════════════════════

  createSandbox(config: SandboxConfig): Promise<Sandbox>;
  getSandbox(id: string): Promise<Sandbox>;
  listSandboxes(): Promise<Sandbox[]>;
  stopSandbox(id: string): Promise<void>;
  resumeSandbox(id: string): Promise<Sandbox>;
  deleteSandbox(id: string): Promise<void>;
  snapshotSandbox(id: string, destination: string): Promise<{ uri: string; sizeBytes: number }>;

  // ═══════════════════════════════════════════════════════════════
  // EXECUTION
  // ═══════════════════════════════════════════════════════════════

  exec(sandboxId: string, command: string, options?: ExecOptions): Promise<ExecResult>;
  prompt(sandboxId: string, message: string, options?: PromptOptions): Promise<PromptResult>;
  task(sandboxId: string, prompt: string, options?: TaskOptions): Promise<TaskResult>;

  // ═══════════════════════════════════════════════════════════════
  // STREAMING
  // ═══════════════════════════════════════════════════════════════

  streamPrompt(sandboxId: string, message: string, options?: PromptOptions): AsyncIterable<AgentEvent>;
  streamTask(sandboxId: string, prompt: string, options?: TaskOptions): AsyncIterable<AgentEvent>;

  // ═══════════════════════════════════════════════════════════════
  // BATCH OPERATIONS
  // ═══════════════════════════════════════════════════════════════

  createBatch(count: number, config: SandboxConfig, options?: BatchOptions): Promise<Batch>;
  runBatchTask(batchOrIds: string | string[], prompt: string, options?: BatchTaskOptions): Promise<BatchResult>;
  deleteBatch(batchId: string): Promise<void>;

  // ═══════════════════════════════════════════════════════════════
  // WORKFLOWS
  // ═══════════════════════════════════════════════════════════════

  createWorkflow(definition: WorkflowDefinition): Promise<Workflow>;
  triggerWorkflow(workflowId: string, input?: Record<string, unknown>): Promise<WorkflowRun>;
  getWorkflowRun(runId: string): Promise<WorkflowRun>;
  cancelWorkflow(runId: string): Promise<void>;

  // ═══════════════════════════════════════════════════════════════
  // SSH
  // ═══════════════════════════════════════════════════════════════

  getSSHCredentials(sandboxId: string): Promise<SSHCredentials>;

  // ═══════════════════════════════════════════════════════════════
  // OPERATORS (decentralized mode only)
  // ═══════════════════════════════════════════════════════════════

  listOperators(filter?: OperatorFilter): Promise<Operator[]>;
  getOperatorPricing(operatorAddress: string): Promise<OperatorPricing>;
}
```

## What NOT to Expose

### Explicitly Private (Never export)

```typescript
// ❌ DO NOT EXPORT

// Rate limiting internals
TIER_LIMITS
getTierLimits()
calculateRateLimit()

// Billing formulas
calculateUsageCost()
CREDIT_CONVERSION_RATE

// Session internals
SESSION_JWT_CLAIMS_SCHEMA
generateSessionToken()  // internal HMAC details

// Webhook signing
generateWebhookSignature()
WEBHOOK_SIGNING_SECRET

// Analytics algorithms
RECOMMENDATION_WEIGHTS
FAILURE_CLASSIFICATION_RULES
SIGNAL_DETECTION_PATTERNS

// AI classifier prompts
ANALYST_SYSTEM_PROMPT
CLASSIFIER_PROMPTS

// Internal orchestration
ContainerDriver
HostProvisioner
AutoscalerDecision
```

### Internal-Only Packages

These packages should never be published to npm:

```
@internal/sdk-algorithms     # Scoring, classification, detection
@internal/sdk-billing        # Billing calculation internals
@internal/sdk-orchestration  # Container/host management
@internal/sdk-analytics      # Usage analytics internals
```

## Security Considerations

### 1. Code Obfuscation (Optional)

For proprietary algorithms that must ship to customers:

```javascript
// tsup.config.ts
export default {
  minify: true,
  terserOptions: {
    mangle: {
      properties: {
        regex: /^_private_/  // Mangle properties starting with _private_
      }
    }
  }
};
```

### 2. Environment-Based Configuration

Move sensitive defaults to runtime config, not compile-time constants:

```typescript
// Instead of:
const TIER_LIMITS = { free: { rps: 5 }, pro: { rps: 100 } };

// Use:
const getTierLimits = () => JSON.parse(process.env.TIER_LIMITS_CONFIG || '{}');
```

### 3. Server-Side Validation

Never trust client-side SDK for:
- Rate limiting enforcement
- Billing calculations
- Access control decisions

The SDK should only provide convenience methods; actual enforcement happens server-side.

### 4. API Key Scoping

Public SDK should support scoped API keys:

```typescript
interface ApiKeyScope {
  sandboxes: 'read' | 'write' | 'admin';
  billing: 'read' | 'write' | 'admin';
  workflows: 'read' | 'write' | 'admin';
}

// Customer creates key with limited scope
const key = await client.createApiKey({
  name: 'ci-runner',
  scopes: {
    sandboxes: 'write',
    billing: 'read',
    workflows: 'read'
  }
});
```

## Implementation Plan

### Phase 1: Create Public SDK Package

1. Create `packages/sdk-public/`
2. Define public type definitions (this document)
3. Implement `UnifiedClient` wrapping existing clients
4. Export only curated APIs

### Phase 2: Refactor Sensitive Packages

1. Move `TIER_LIMITS` to runtime config in `sdk-service`
2. Extract scoring algorithms to `@internal/sdk-algorithms`
3. Create facade classes for customer/billing services
4. Remove direct exports of internal functions

### Phase 3: Add Blueprint Client

1. Implement `BlueprintClient` for decentralized mode
2. Add operator discovery methods
3. Wire up x402 payment handling
4. Test with local Anvil + blueprint

### Phase 4: Documentation & Publishing

1. Generate API docs from public types
2. Create migration guide from internal packages
3. Set up npm publish pipeline for `@tangle-network/sdk-public`
4. Deprecation notices on internal packages

## Package Dependency Graph

```
@tangle-network/sdk-public (PUBLIC - npm published)
    │
    ├── Uses types only from:
    │   ├── agent-interface (PUBLIC - provider contract)
    │   └── sdk-core/types (PUBLIC - transport types)
    │
    └── Wraps at runtime:
        ├── sdk (INTERNAL - orchestrator client)
        ├── sdk-service (INTERNAL - facades only)
        └── Blueprint contract (on-chain)

@internal/* packages (NEVER published)
    ├── sdk-algorithms
    ├── sdk-billing-internals
    └── sdk-orchestration
```

## Summary

| Category | Action |
|----------|--------|
| **Create** | `@tangle-network/sdk-public` with curated exports |
| **Publish** | Public SDK + `agent-interface` (versioned contract) |
| **Keep Internal** | Full `sdk`, `sdk-service`, `sdk-data` packages |
| **Never Publish** | Algorithms, billing formulas, orchestration internals |
| **Refactor** | Move constants to runtime config, create facades |

This design ensures customers get a clean, documented API surface while proprietary business logic remains protected.
