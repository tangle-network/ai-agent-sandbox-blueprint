# AI Agent Sandbox Blueprint — Per-Job Pricing Rationale

This document explains the pricing model for the AI Agent Sandbox Blueprint, including competitor analysis, raw cost breakdowns, and the methodology behind each job type's pricing tier.

## Pricing Model Overview

The blueprint uses a **multiplier-based pricing model** for its **5 on-chain jobs**. The blueprint owner sets a single **base rate** (the cost of the cheapest on-chain operation), and all job types are priced as multiples of that base rate. Operations that do not mutate on-chain state (exec, prompt, task, stop, resume, snapshot, SSH, batch) are served via the off-chain operator HTTP API and are not priced as on-chain jobs.

This design:
- Adapts automatically to token price changes (just adjust the base rate)
- Maintains correct cost ratios between job types
- Is simple for operators to reason about
- Can be reconfigured via `setJobEventRates()` on the Tangle contract

### On-Chain Job Pricing (5 jobs)

These are the only operations priced as on-chain jobs. Each job **must** mutate authoritative state.

| Mult | Job | ID | Rationale |
|------|-----|----|-----------|
| 1x | SANDBOX_DELETE | 1 | Trivial teardown |
| 1x | WORKFLOW_CANCEL | 4 | Flag update |
| 2x | WORKFLOW_CREATE | 2 | Config validation + storage |
| 5x | WORKFLOW_TRIGGER | 3 | Initiates execution pipeline |
| 50x | SANDBOX_CREATE | 0 | Container lifecycle + prepaid runtime |

### Off-Chain Operations (operator API)

These operations are served via the authenticated operator HTTP API and are **not** on-chain jobs.
Operators set their own pricing for these via RFQ or subscription models:

- **Trivial** (<100ms): exec, stop, resume, SSH revoke
- **Light**: SSH provision, secret injection
- **I/O-heavy**: snapshot
- **LLM**: prompt (single inference), task (multi-turn agent)
- **Batch**: batch exec, batch create, batch task, batch collect

### Example Pricing (base rate = 0.001 TNT, assuming 1 TNT ≈ $1)

| Job | Rate (TNT) | Rate (USD) |
|-----|-----------|-----------|
| SANDBOX_DELETE | 0.001 | $0.001 |
| WORKFLOW_CREATE | 0.002 | $0.002 |
| WORKFLOW_TRIGGER | 0.005 | $0.005 |
| SANDBOX_CREATE | 0.050 | $0.050 |

---

## Competitor Analysis

### AI Sandbox Platforms

| Platform | Model | 1 vCPU + 2GB/hr | Key Differentiator |
|----------|-------|-----------------|-------------------|
| **E2B** | Per-second compute | $0.083/hr | AI-native, ~150ms cold start |
| **Daytona** | Per-second compute | $0.083/hr | ~90ms cold start |
| **Modal** | Per-second + sandbox premium | $0.119/hr | 3x sandbox multiplier |
| **CodeSandbox** | Credit-based | $0.074/hr (Pico) | SDK-first |
| **GitHub Codespaces** | Per-hour | $0.180/hr (2-core) | Full dev environment |
| **Gitpod** | Credit-based | $0.160/hr (Standard) | Kubernetes-native |
| **Vercel Sandbox** | Per-resource | $0.128/CPU-hr | $0.60/million creations |
| **Together AI** | Per-resource | $0.045/vCPU-hr | Code Interpreter $0.03/session |
| **Replit** | Effort-based | $0.36/hr compute | Agent tasks $0.25-$10+ |
| **Fly.io** | Per-second | $0.045/hr (perf) | Lowest raw compute |
| **Northflank** | Per-second | $0.033/hr | Kata container isolation |

### Key Insights from Competitors

1. **Per-second billing is standard** — all modern platforms bill per-second, not per-hour
2. **AI agent tasks are priced much higher** than raw compute — Replit charges $0.25-$10+ per agent task
3. **Container creation is essentially free** — only Vercel charges for it ($0.60/million = $0.0000006 each)
4. **LLM costs dominate** for prompt/task jobs — compute is a rounding error next to inference costs

---

## Raw Infrastructure Costs

### Container Compute (per hour, 2 vCPU + 4GB RAM)

| Provider | Type | $/hour |
|----------|------|--------|
| Hetzner Bare Metal (multi-tenant) | Self-managed | $0.011 |
| Hetzner Cloud (CPX21) | Cloud VM | $0.012 |
| GCP Compute Engine (e2-medium) | Cloud VM | $0.034 |
| AWS Fargate | Managed | $0.099 |
| AWS EC2 (c6a.xlarge, shared) | Self-managed | $0.077 |

**Median operator cost: ~$0.05/hr for 2 vCPU + 4GB RAM**

### Container Creation Overhead

| Component | Cost |
|-----------|------|
| Cold start (image pull + startup) | ~$0.001 |
| CPU burst during init | < $0.0001 |
| Total per creation | **~$0.001** |

### Command Execution (EXEC)

| Operation | CPU Time | Cost |
|-----------|---------|------|
| Simple bash (ls, echo, cat) | 50-200ms | $0.000001 |
| Moderate (grep, pip small) | 1-5s | $0.00005 |
| 100 mixed commands | 60-120s total | $0.001 |
| **Per-command average** | ~1s | **$0.00001** |

### Snapshot (Docker Commit)

| Size | Time | Storage/month |
|------|------|--------------|
| Small delta (<100MB) | 1-5s | $0.005 |
| Moderate (~300MB) | 5-15s | $0.015 |
| Large (1GB+) | 15-60s | $0.050 |
| **Typical snapshot** | 5-10s | **$0.01** |

### SSH Session

| Component | Cost/hour |
|-----------|----------|
| Keepalive bandwidth | $0.00 |
| Active terminal I/O (~5MB/hr) | $0.0005 |
| **Total** | **~$0.001** |

---

## LLM Inference Costs

### Per-Call Costs by Model Tier (Single Prompt: ~3K input + 800 output tokens)

| Tier | Models | Cost/call |
|------|--------|-----------|
| Budget | GPT-4o-mini, Gemini Flash, Llama 8B | $0.0002-0.001 |
| Mid | GPT-4o, Claude Sonnet 4.5, Gemini Pro | $0.01-0.02 |
| Premium | Claude Opus, GPT-4 Turbo | $0.03-0.05 |

### Per-Task Costs by Model Tier (7-turn agent: ~70K input + 5.6K output tokens)

| Tier | Models | Cost/task | With Caching |
|------|--------|-----------|-------------|
| Budget | GPT-4o-mini, Gemini Flash | $0.009-0.014 | $0.005-0.007 |
| Mid | GPT-4o, Claude Sonnet 4.5 | $0.14-0.29 | $0.08-0.12 |
| Premium | Claude Opus | $0.49 | $0.17 |

### Token Accumulation in Multi-Turn Tasks

| Turn | Input Tokens | Output Tokens | Cumulative |
|------|-------------|---------------|-----------|
| 1 | 3,000 | 800 | 3,800 |
| 3 | 8,000 | 800 | 8,800 |
| 5 | 12,000 | 800 | 12,800 |
| 7 | 18,000 | 800 | 18,800 |
| 10 | 25,000 | 800 | 25,800 |
| **Total (7 turns)** | **~70,000** | **~5,600** | **75,600** |

---

## Per-Job Pricing Derivation (On-Chain Jobs Only)

Only state-changing operations are on-chain jobs. Off-chain operations (exec, prompt, task,
stop, resume, snapshot, SSH, batch) are priced by operators via their own billing models.

### 1x — Trivial Teardown / Flag Updates

**On-chain jobs:** SANDBOX_DELETE, WORKFLOW_CANCEL

**Cost basis:**
- Raw cost: $0.00001-0.0001 per operation
- Single state flag update or container removal
- <100ms CPU, no external calls

### 2x — Config Validation + Storage

**On-chain jobs:** WORKFLOW_CREATE

**Cost basis:**
- Workflow config validation + on-chain storage: ~$0.0002
- Slightly more compute than a trivial delete

### 5x — Execution Pipeline Trigger

**On-chain jobs:** WORKFLOW_TRIGGER

**Cost basis:**
- Initiates sandbox creation + task execution pipeline
- Triggers downstream workflow execution on operators

### 50x — Container Lifecycle

**On-chain jobs:** SANDBOX_CREATE

**Cost basis:**
- Container creation + 10min prepaid runtime:
  - AWS Fargate: $0.001 creation + $0.008 (10min runtime) = $0.009
  - E2B: $0.014 (10min at 1vCPU+2GB)
  - Plus operator margin (40%): $0.013-0.020
- **50x base at $0.001 = $0.05** — covers creation + short prepaid window with margin

**Competitive positioning:**
- E2B 10min session: $0.014
- GitHub Codespaces 10min: $0.030
- Our rate ($0.05) includes creation overhead and operator margin

---

## Operator Economics

### Margin Analysis

| Cost Component | % of Revenue |
|---------------|-------------|
| Compute infrastructure | 30-40% |
| LLM inference (for prompt/task jobs) | 20-40% |
| Network + storage | 5-10% |
| **Operator margin** | **20-40%** |

### Breakeven Analysis (mid-tier operator, Hetzner infrastructure)

| Monthly Volume | Revenue | Infra Cost | Net Margin |
|---------------|---------|-----------|-----------|
| 1K SANDBOX_CREATE | $50 | $12 | $38 (76%) |
| 5K WORKFLOW_TRIGGER | $25 | $3 | $22 (88%) |
| 1K SANDBOX_DELETE | $1 | $0.10 | $0.90 (90%) |

### RFQ System for Custom Pricing

For jobs where costs vary significantly (different LLM models, different container specs), operators can use the **Job RFQ system** (`submitJobFromQuote`) to quote custom prices per job. This is recommended for:

- TASK jobs with premium models (Claude Opus → quote higher)
- SANDBOX_CREATE with custom specs (16 vCPU → quote higher)
- BATCH operations with large N (volume discount → quote lower)

---

## Configuration Guide

### Setting Job Rates

After registering your blueprint on Tangle, configure rates using the `ConfigureJobRates.s.sol` script:

```bash
# Base rate determines all job prices via multipliers
# Adjust based on your token's USD value
BASE_RATE=1000000000000000 \     # 0.001 TNT
BLUEPRINT_ID=<your-id> \
TANGLE_ADDRESS=<tangle-proxy> \
BSM_ADDRESS=<your-bsm> \
forge script contracts/script/ConfigureJobRates.s.sol:ConfigureJobRates \
  --rpc-url $RPC_URL --broadcast
```

### Base Rate Guidelines

| If 1 TNT ≈ | Set BASE_RATE to | DELETE cost (1x) | CREATE cost (50x) |
|------------|-----------------|-------------------|-------------------|
| $0.01 | 1e17 (0.1 TNT) | $0.001 | $0.05 |
| $0.10 | 1e16 (0.01 TNT) | $0.001 | $0.05 |
| $1.00 | 1e15 (0.001 TNT) | $0.001 | $0.05 |
| $10.00 | 1e14 (0.0001 TNT) | $0.001 | $0.05 |

### Overriding Individual Rates

Blueprint owners can override any individual rate by calling `setJobEventRates()` directly on the Tangle contract with a subset of job indexes (0-6):

```solidity
// Make SANDBOX_CREATE cheaper (e.g., lightweight containers)
uint8[] memory jobs = new uint8[](1);
uint256[] memory rates = new uint256[](1);
jobs[0] = 0; // JOB_SANDBOX_CREATE
rates[0] = 25 * baseRate; // 25x instead of default 50x
tangle.setJobEventRates(blueprintId, jobs, rates);
```
