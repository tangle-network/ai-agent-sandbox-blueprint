//! Micro-benchmarks for the HTTP helpers used on every sidecar call:
//! URL construction and auth-header building.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

use sandbox_runtime::http::{auth_headers, build_url};

fn bench_build_url(c: &mut Criterion) {
    let mut group = c.benchmark_group("http/build_url");
    let cases = [
        ("simple", ("http://localhost:8080", "/api/test")),
        (
            "nested",
            ("https://example.com:9443", "/v1/sandboxes/create"),
        ),
        ("path_prefix", ("http://localhost:8080/prefix/", "api/test")),
    ];
    for (name, (base, path)) in cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), &(base, path), |b, v| {
            b.iter(|| black_box(build_url(black_box(v.0), black_box(v.1))));
        });
    }
    group.finish();
}

fn bench_auth_headers(c: &mut Criterion) {
    let mut group = c.benchmark_group("http/auth_headers");
    group.bench_function("short_token", |b| {
        b.iter(|| black_box(auth_headers(black_box("abc123"))));
    });
    group.bench_function("paseto_like_token", |b| {
        let token = "v4.local.aZbN3PZHVjqmCrSvBBZF_s3c9lXL3L21WJztcxHnFj_RFakRPBKjZCROQ5s0Sbp2";
        b.iter(|| black_box(auth_headers(black_box(token))));
    });
    group.finish();
}

criterion_group!(http_benches, bench_build_url, bench_auth_headers);
criterion_main!(http_benches);
