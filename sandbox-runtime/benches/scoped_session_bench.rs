//! Micro-benchmarks for the scoped session auth service.
//!
//! `resolve_bearer` runs on every instance-mode API request and performs an
//! unconditional full-map GC. Measure how badly this scales with session count.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use sandbox_runtime::scoped_session_auth::{
    ScopedAuthConfig, ScopedAuthMode, ScopedAuthResource, ScopedAuthService,
};

fn make_service(session_count: usize) -> (ScopedAuthService, Vec<String>) {
    let service = ScopedAuthService::new(ScopedAuthConfig {
        access_token: Some("shared-token".to_string()),
        max_sessions: session_count.max(1) * 2,
        max_challenges: 100_000,
        ..ScopedAuthConfig::default()
    });
    let mut tokens = Vec::with_capacity(session_count);
    for i in 0..session_count {
        let resource = ScopedAuthResource {
            scope_id: format!("inst-{i}"),
            owner: format!("0x{:040x}", i + 1),
            auth_mode: ScopedAuthMode::AccessToken,
        };
        let session = service
            .create_access_token_session(&resource, "shared-token")
            .expect("create session");
        tokens.push(session.token);
    }
    (service, tokens)
}

fn bench_resolve_bearer_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("scoped_session/resolve_bearer");
    group.throughput(Throughput::Elements(1));

    for &n in &[1usize, 100, 1_000, 10_000] {
        let (service, tokens) = make_service(n);
        let mut idx = 0usize;
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let token = &tokens[idx % tokens.len()];
                idx = idx.wrapping_add(1);
                let claims = service.resolve_bearer(black_box(token));
                black_box(claims);
            })
        });
    }
    group.finish();
}

fn bench_create_access_token_session(c: &mut Criterion) {
    let service = ScopedAuthService::new(ScopedAuthConfig {
        access_token: Some("shared-token".to_string()),
        max_sessions: 1_000_000,
        max_challenges: 100_000,
        ..ScopedAuthConfig::default()
    });
    let mut i = 0u64;
    c.bench_function("scoped_session/create_access_token", |b| {
        b.iter(|| {
            let resource = ScopedAuthResource {
                scope_id: format!("bench-{i}"),
                owner: format!("0x{:040x}", i + 1),
                auth_mode: ScopedAuthMode::AccessToken,
            };
            i = i.wrapping_add(1);
            let s = service.create_access_token_session(&resource, "shared-token");
            black_box(s.expect("create"));
        })
    });
}

criterion_group!(
    scoped_session_benches,
    bench_resolve_bearer_scaling,
    bench_create_access_token_session,
);
criterion_main!(scoped_session_benches);
