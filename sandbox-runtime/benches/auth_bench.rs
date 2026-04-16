//! Micro-benchmarks for token generation primitives.
//!
//! Scope: functions called during sandbox creation and each authentication
//! challenge response.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use sandbox_runtime::auth::{generate_token, require_sidecar_token, token_from_request};

fn bench_generate_token(c: &mut Criterion) {
    c.bench_function("auth/generate_token", |b| {
        b.iter(|| {
            let t = generate_token();
            black_box(t);
        })
    });
}

fn bench_token_from_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth/token_from_request");
    group.bench_function("empty_generates_new", |b| {
        b.iter(|| black_box(token_from_request(black_box(""))))
    });
    group.bench_function("provided_passthrough", |b| {
        let provided = "  abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  ";
        b.iter(|| black_box(token_from_request(black_box(provided))))
    });
    group.finish();
}

fn bench_require_sidecar_token(c: &mut Criterion) {
    let token = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    c.bench_function("auth/require_sidecar_token", |b| {
        b.iter(|| black_box(require_sidecar_token(black_box(token))))
    });
}

criterion_group!(
    auth_benches,
    bench_generate_token,
    bench_token_from_request,
    bench_require_sidecar_token
);
criterion_main!(auth_benches);
