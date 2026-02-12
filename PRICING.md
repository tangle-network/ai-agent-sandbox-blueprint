# AI Agent Sandbox Blueprint — Per-Job Pricing Rationale

This document explains the pricing model for the AI Agent Sandbox Blueprint, including competitor analysis, raw cost breakdowns, and the methodology behind each job type's pricing tier.

## Pricing Model Overview

The blueprint uses a **multiplier-based pricing model**. The blueprint owner sets a single **base rate** (the cost of the cheapest operation, `EXEC`), and all 17 job types are priced as multiples of that base rate.

This design:
- Adapts automatically to token price changes (just adjust the base rate)
- Maintains correct cost ratios between job types
- Is simple for operators to reason about
- Can be reconfigured via `setJobEventRates()` on the Tangle contract

### Quick Reference

| Tier | Mult | Jobs | Rationale |
|------|------|------|-----------|
| 1 | 1x | EXEC, STOP, RESUME, DELETE, BATCH_COLLECT, WORKFLOW_CANCEL, SSH_REVOKE | Trivial ops, <100ms CPU |
| 2 | 2x | SSH_PROVISION, WORKFLOW_CREATE | Light state changes |
| 3 | 5x | SNAPSHOT, WORKFLOW_TRIGGER | I/O-heavy operations |
| 4 | 20x | PROMPT | Single LLM inference call |
| 5 | 50x | SANDBOX_CREATE, BATCH_EXEC | Container lifecycle |
| 6 | 100x | BATCH_CREATE | Batch container creation |
| 7 | 250x | TASK | Multi-turn AI agent |
| 8 | 500x | BATCH_TASK | Batch agent tasks |

### Example Pricing (base rate = 0.001 TNT, assuming 1 TNT ≈ $1)

| Job | Rate (TNT) | Rate (USD) |
|-----|-----------|-----------|
| EXEC | 0.001 | $0.001 |
| SANDBOX_CREATE | 0.050 | $0.050 |
| PROMPT | 0.020 | $0.020 |
| TASK | 0.250 | $0.250 |
| BATCH_TASK | 0.500 | $0.500 |

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

## Per-Job Pricing Derivation

### Tier 1 (1x) — Trivial Operations

**Jobs:** EXEC, STOP, RESUME, DELETE, BATCH_COLLECT, WORKFLOW_CANCEL, SSH_REVOKE

**Cost basis:**
- Raw cost: $0.00001-0.0001 per operation
- Single `docker exec` or state flag update
- <100ms CPU, no external calls, no I/O

**Competitive positioning:** Below E2B's per-second rate ($0.000014/vCPU/s × 0.1s = $0.0000014). We charge slightly more because each job submission has on-chain transaction overhead.

### Tier 2 (2x) — Light State Changes

**Jobs:** SSH_PROVISION, WORKFLOW_CREATE

**Cost basis:**
- SSH key generation: ~$0.0002 (CPU burst for key derivation)
- Workflow config validation + storage: ~$0.0002
- Slightly more compute than a trivial exec

### Tier 3 (5x) — I/O-Heavy Operations

**Jobs:** SNAPSHOT, WORKFLOW_TRIGGER

**Cost basis:**
- Snapshot: Docker commit (~300MB), 5-15s of disk I/O, $0.001-0.002 compute + $0.01/month storage
- Workflow trigger: Initiates sandbox creation + task execution pipeline
- Competitors: Vercel charges $0.60/million creations; our 5x rate is still very competitive

### Tier 4 (20x) — Single LLM Call

**Jobs:** PROMPT

**Cost basis:**
- Budget models: $0.001/call → 10x base rate would cover it
- Mid-tier models: $0.015/call → 15x needed
- Premium models: $0.035/call → 35x needed
- **20x is the geometric mean**, covering mid-tier models with margin

**Competitive positioning:**
- Together AI code sandbox: $0.03/session (entire session, not per-call)
- Our 20x at $0.001 base = $0.02/prompt — competitive with mid-tier models

### Tier 5 (50x) — Container Lifecycle

**Jobs:** SANDBOX_CREATE, BATCH_EXEC

**Cost basis:**
- Container creation + 10min prepaid runtime:
  - AWS Fargate: $0.001 creation + $0.008 (10min runtime) = $0.009
  - E2B: $0.014 (10min at 1vCPU+2GB)
  - Plus operator margin (40%): $0.013-0.020
- **50x base at $0.001 = $0.05** — covers creation + short prepaid window with margin

**Competitive positioning:**
- E2B 10min session: $0.014
- GitHub Codespaces 10min: $0.030
- Our rate ($0.05) includes creation overhead and operator margin — premium for on-demand, no subscription required

### Tier 6 (100x) — Batch Container Creation

**Jobs:** BATCH_CREATE

**Cost basis:**
- Multiple sandbox creations in one transaction
- Priced at 2x SANDBOX_CREATE to reflect batch overhead + coordination cost
- Actual per-sandbox cost is lower due to amortized setup

### Tier 7 (250x) — Multi-Turn AI Agent

**Jobs:** TASK

**Cost basis:**
- 7-turn average with token accumulation
- Budget model: $0.01, Mid: $0.10 (cached), Premium: $0.17 (cached)
- Plus compute time (30-300s active): $0.001-0.01
- **250x base at $0.001 = $0.25** — covers mid-tier model with caching + margin

**Competitive positioning:**
- Replit agent tasks: $0.25-$10+ (effort-based)
- Our $0.25 is at the low end of Replit's range
- Operators can use RFQ system to quote higher for premium model tasks

### Tier 8 (500x) — Batch Agent Tasks

**Jobs:** BATCH_TASK

**Cost basis:**
- Multiple agent tasks in one submission
- 2x TASK rate to reflect coordination + parallel execution overhead
- Operators may process these across multiple containers concurrently

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

| Monthly Volume | Revenue | Infra Cost | LLM Cost | Net Margin |
|---------------|---------|-----------|----------|-----------|
| 10K EXEC jobs | $10 | $1 | $0 | $9 (90%) |
| 1K SANDBOX_CREATE | $50 | $12 | $0 | $38 (76%) |
| 1K PROMPT jobs | $20 | $1 | $15 | $4 (20%) |
| 500 TASK jobs | $125 | $5 | $50 | $70 (56%) |

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

| If 1 TNT ≈ | Set BASE_RATE to | EXEC cost | CREATE cost | TASK cost |
|------------|-----------------|-----------|-------------|-----------|
| $0.01 | 1e17 (0.1 TNT) | $0.001 | $0.05 | $0.25 |
| $0.10 | 1e16 (0.01 TNT) | $0.001 | $0.05 | $0.25 |
| $1.00 | 1e15 (0.001 TNT) | $0.001 | $0.05 | $0.25 |
| $10.00 | 1e14 (0.0001 TNT) | $0.001 | $0.05 | $0.25 |

### Overriding Individual Rates

Blueprint owners can override any individual rate by calling `setJobEventRates()` directly on the Tangle contract with a subset of job indexes:

```solidity
// Make TASK jobs cheaper (e.g., you use budget LLMs)
uint8[] memory jobs = new uint8[](1);
uint256[] memory rates = new uint256[](1);
jobs[0] = 12; // JOB_TASK
rates[0] = 100 * baseRate; // 100x instead of default 250x
tangle.setJobEventRates(blueprintId, jobs, rates);
```
