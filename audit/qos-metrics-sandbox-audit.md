# Security Audit Report: Sandbox Blueprint Metrics Integration

**Scope**: Metrics collection, session tracking, resource tracking, global static safety, overflow potential, and QoS background task wiring.
**Date**: 2026-02-08
**Auditor**: Claude Opus 4.6

**Files Analyzed**:
- `ai-agent-sandbox-blueprint-lib/src/metrics.rs`
- `ai-agent-sandbox-blueprint-bin/src/main.rs`
- `ai-agent-sandbox-blueprint-lib/src/jobs/exec.rs`
- `ai-agent-sandbox-blueprint-lib/src/jobs/sandbox.rs`
- `ai-agent-sandbox-blueprint-lib/src/jobs/batch.rs`
- `ai-agent-sandbox-blueprint-lib/src/workflows.rs`

---

## Finding 1: TOCTOU Race in `record_sandbox_deleted` -- Severity: MEDIUM

**Location**: `metrics.rs`, lines 84-95.

The underflow guard uses a load-then-subtract pattern that is not atomic as a unit:

```rust
self.active_sandboxes
    .fetch_sub(1.min(self.active_sandboxes.load(Ordering::Relaxed)), Ordering::Relaxed);
```

Between the `load()` and the `fetch_sub()`, another thread can decrement the same counter, causing unsigned underflow to `u64::MAX`.

**Fix**: Use `fetch_update`:
```rust
self.active_sandboxes.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
    if v > 0 { Some(v - 1) } else { Some(0) }
});
```

---

## Finding 2: TOCTOU Race in `session_end` -- Severity: MEDIUM

**Location**: `metrics.rs`, lines 103-108.

Same pattern as Finding 1. Two concurrent `session_end` calls where `active_sessions == 1` will both subtract 1, causing underflow to `u64::MAX`.

---

## Finding 3: Session Counter Leak on Task Cancellation -- Severity: LOW

**Location**: `exec.rs` lines 131-144, `workflows.rs` lines 200-213.

If the task is cancelled via `tokio::task::abort()` between `session_start()` and `session_end()`, the session counter is permanently incremented. A `Drop`-based RAII guard would fix this.

---

## Finding 4: Resource Counters Permanently Inflated -- Severity: MEDIUM

**Location**: `sandbox.rs` lines 55-57.

```rust
crate::metrics::metrics().record_sandbox_deleted(0, 0);
```

`SandboxRecord` does not store `cpu_cores` or `memory_mb`, so `(0, 0)` is passed on deletion. `allocated_cpu_cores` and `allocated_memory_mb` are never decremented, making them cumulative totals rather than current allocations. The metric names and comments imply current allocation.

**Fix**: Store `cpu_cores` and `memory_mb` in `SandboxRecord`.

---

## Finding 5: Batch Create Skips Metrics -- Severity: LOW

**Location**: `batch.rs` lines 38-46.

`batch_create` calls `create_sidecar()` but does not call `record_sandbox_created()`. Sandboxes created via batch are invisible to metrics.

---

## Finding 6: Relaxed Ordering -- Severity: INFO (Acceptable)

All atomic operations use `Ordering::Relaxed`. Correct for approximate gauges/counters. Minor note: `peak_sandboxes` tracking may slightly undercount in concurrent creation scenarios.

---

## Finding 7: u64 Overflow of Token Counters -- Severity: INFO

`total_input_tokens` and `total_duration_ms` accumulate across all jobs. Even at unrealistic rates, overflow would take hundreds of thousands of years. Not practically achievable.

---

## Finding 8: QoS Provider Lifetime and Background Task -- Severity: LOW

`qos_service` is dropped after the builder block. If `provider()` returns something dependent on the service's background task, dropping `qos_service` could break it. No error handling on `add_on_chain_metric`. Spawned task has no shutdown coordination.

---

## Finding 9: Snapshot Non-Atomic Average -- Severity: INFO

`total_jobs` and `total_duration_ms` are loaded in separate operations. A job completing between loads causes slight average discrepancy. Acceptable.

---

## Finding 10: Global Static Thread Safety -- Severity: INFO (Sound)

`OnChainMetrics` with `AtomicU64` fields in a `static` is correct and idiomatic. No issues.

---

## Summary Table

| # | Finding | Severity | Category |
|---|---------|----------|----------|
| 1 | TOCTOU race in `record_sandbox_deleted` | MEDIUM | Concurrency |
| 2 | TOCTOU race in `session_end` | MEDIUM | Concurrency |
| 3 | Session counter leak on task cancellation | LOW | Resource Leak |
| 4 | Resource counters never decremented | MEDIUM | Data Correctness |
| 5 | `batch_create` skips metrics | LOW | Data Correctness |
| 6 | Relaxed ordering acceptable | INFO | Concurrency |
| 7 | u64 overflow not reachable | INFO | Overflow |
| 8 | QoS provider lifetime / error handling | LOW | Reliability |
| 9 | Snapshot average non-atomic | INFO | Concurrency |
| 10 | Global static is thread-safe | INFO | Concurrency |

---

## Recommendations (Priority Order)

1. **Fix TOCTOU races** (Findings 1, 2): Use `fetch_update` for atomic check-and-decrement.
2. **Store resources in SandboxRecord** (Finding 4): Enable accurate resource counter decrements.
3. **Add metrics to batch_create** (Finding 5).
4. **RAII session guard** (Finding 3): Prevents leaks on task cancellation.
5. **Log errors from add_on_chain_metric** (Finding 8).
