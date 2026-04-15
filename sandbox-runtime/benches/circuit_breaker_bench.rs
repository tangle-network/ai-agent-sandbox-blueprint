//! Micro-benchmarks for the per-sandbox circuit breaker.
//!
//! `check_health` runs before every sidecar HTTP call. The harden scan flagged
//! `std::env::var("CIRCUIT_BREAKER_COOLDOWN_SECS")` on every invocation as a
//! potential hot-path cost (C runtime lock in `getenv(3)`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use sandbox_runtime::circuit_breaker::{
    check_health, clear_all_for_testing, mark_healthy, mark_unhealthy,
};

fn bench_check_health_closed(c: &mut Criterion) {
    clear_all_for_testing();
    let mut group = c.benchmark_group("circuit_breaker/closed");
    group.throughput(Throughput::Elements(1));
    group.bench_function("check_health", |b| {
        b.iter(|| {
            let _ = black_box(check_health(black_box("sandbox-bench-closed")));
        })
    });
    group.finish();
}

fn bench_check_health_open(c: &mut Criterion) {
    clear_all_for_testing();
    mark_unhealthy("sandbox-bench-open");
    let mut group = c.benchmark_group("circuit_breaker/open");
    group.throughput(Throughput::Elements(1));
    group.bench_function("check_health", |b| {
        b.iter(|| {
            let _ = black_box(check_health(black_box("sandbox-bench-open")));
        })
    });
    group.finish();
    mark_healthy("sandbox-bench-open");
}

fn bench_mark_transition(c: &mut Criterion) {
    clear_all_for_testing();
    let mut group = c.benchmark_group("circuit_breaker/transition");
    group.throughput(Throughput::Elements(1));
    group.bench_function("unhealthy_then_healthy", |b| {
        b.iter(|| {
            mark_unhealthy(black_box("sandbox-bench-trans"));
            mark_healthy(black_box("sandbox-bench-trans"));
        })
    });
    group.finish();
}

criterion_group!(
    circuit_breaker_benches,
    bench_check_health_closed,
    bench_check_health_open,
    bench_mark_transition,
);
criterion_main!(circuit_breaker_benches);
