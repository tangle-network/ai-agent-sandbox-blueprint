//! Micro-benchmarks for the per-IP sliding-window rate limiter.
//!
//! The limiter's `check()` is called on every HTTP request. The harden scan
//! identified nested locks (outer `buckets` Mutex + inner `last_gc` Mutex) as
//! a potential contention source. These benches measure the single-IP hot
//! path, the multi-IP scaling behavior, and the GC spike latency.

use std::net::{IpAddr, Ipv4Addr};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use sandbox_runtime::rate_limit::{RateLimitConfig, RateLimiter};

fn bench_single_ip(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limit/single_ip");
    group.throughput(Throughput::Elements(1));
    let limiter = RateLimiter::new(RateLimitConfig::new(1_000_000, 60));
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

    group.bench_function("check", |b| {
        b.iter(|| {
            black_box(limiter.check(black_box(ip)));
        })
    });
    group.finish();
}

fn bench_many_ips(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limit/many_ips");
    group.throughput(Throughput::Elements(1));

    for ip_count in [100usize, 1_000, 10_000] {
        let limiter = RateLimiter::new(RateLimitConfig::new(1_000_000, 60));
        for i in 0..ip_count {
            let ip = IpAddr::V4(Ipv4Addr::new(
                10,
                ((i >> 16) & 0xFF) as u8,
                ((i >> 8) & 0xFF) as u8,
                (i & 0xFF) as u8,
            ));
            let _ = limiter.check(ip);
        }
        let ips: Vec<IpAddr> = (0..1024)
            .map(|i| {
                IpAddr::V4(Ipv4Addr::new(
                    10,
                    ((i >> 16) & 0xFF) as u8,
                    ((i >> 8) & 0xFF) as u8,
                    (i & 0xFF) as u8,
                ))
            })
            .collect();
        let mut idx = 0usize;
        group.bench_with_input(
            BenchmarkId::from_parameter(ip_count),
            &ip_count,
            |b, _| {
                b.iter(|| {
                    let ip = ips[idx % ips.len()];
                    idx = idx.wrapping_add(1);
                    black_box(limiter.check(black_box(ip)));
                })
            },
        );
    }
    group.finish();
}

fn bench_window_at_cap(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limit/window_full");
    group.throughput(Throughput::Elements(1));

    let limiter = RateLimiter::new(RateLimitConfig::new(2_400, 60));
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99));
    for _ in 0..2_400 {
        let _ = limiter.check(ip);
    }
    group.bench_function("check_at_cap", |b| {
        b.iter(|| {
            black_box(limiter.check(black_box(ip)));
        })
    });
    group.finish();
}

criterion_group!(
    rate_limit_benches,
    bench_single_ip,
    bench_many_ips,
    bench_window_at_cap,
);
criterion_main!(rate_limit_benches);
