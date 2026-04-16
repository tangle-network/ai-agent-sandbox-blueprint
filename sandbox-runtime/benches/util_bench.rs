//! Micro-benchmarks for the util helpers: snapshot command construction,
//! shell escaping, and JSON object parsing.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

use sandbox_runtime::util::{build_snapshot_command, parse_json_object, shell_escape};

fn bench_shell_escape(c: &mut Criterion) {
    let mut group = c.benchmark_group("util/shell_escape");
    for (name, input) in [
        ("short_safe", "safe-string"),
        ("with_quotes", "it's a 'quote-heavy' string"),
        ("shell_meta", "hello; rm -rf /; echo `id`; $(whoami)"),
        ("long", "a".repeat(1024).as_str()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), input, |b, s| {
            b.iter(|| black_box(shell_escape(black_box(s))));
        });
    }
    group.finish();
}

fn bench_build_snapshot_command(c: &mut Criterion) {
    let mut group = c.benchmark_group("util/build_snapshot_command");

    let cases = [
        ("https_public_ip", "https://93.184.216.34/snap.tar.gz"),
        ("s3_url", "s3://my-bucket/snapshots/2026/04/snap.tar.gz"),
        (
            "ipv6_public",
            "https://[2607:f8b0:4004:800::200e]/snap.tar.gz",
        ),
    ];
    for (name, dest) in cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), dest, |b, d| {
            b.iter(|| black_box(build_snapshot_command(black_box(d), true, true)));
        });
    }
    group.finish();
}

fn bench_snapshot_rejects(c: &mut Criterion) {
    // Rejection paths matter too — cost of running SSRF checks on bad input.
    let mut group = c.benchmark_group("util/snapshot_rejects");
    let cases = [
        ("private_ipv4", "https://10.0.0.5/snap"),
        ("loopback_ipv4", "https://127.0.0.1/snap"),
        ("metadata_ipv4", "https://169.254.169.254/snap"),
        ("dns_hostname", "https://attacker.com/snap"),
        ("http_scheme", "http://93.184.216.34/snap"),
        ("file_scheme", "file:///etc/passwd"),
        ("ipv4_mapped_ipv6", "https://[::ffff:10.0.0.1]/snap"),
    ];
    for (name, dest) in cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), dest, |b, d| {
            b.iter(|| black_box(build_snapshot_command(black_box(d), true, true)));
        });
    }
    group.finish();
}

fn bench_parse_json_object(c: &mut Criterion) {
    let mut group = c.benchmark_group("util/parse_json_object");
    let small = r#"{"key":"value","n":42}"#;
    let medium: String = {
        let mut s = String::from("{");
        for i in 0..100 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"key_{i}\":\"val_{i}\""));
        }
        s.push('}');
        s
    };
    group.bench_function("small_object", |b| {
        b.iter(|| black_box(parse_json_object(black_box(small), "env")))
    });
    group.bench_function("100_keys", |b| {
        b.iter(|| black_box(parse_json_object(black_box(medium.as_str()), "env")))
    });
    group.finish();
}

criterion_group!(
    util_benches,
    bench_shell_escape,
    bench_build_snapshot_command,
    bench_snapshot_rejects,
    bench_parse_json_object,
);
criterion_main!(util_benches);
