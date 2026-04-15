//! Micro-benchmarks for the primitives used by at-rest secret encryption.
//!
//! `seal_field` / `unseal_field` are private wrappers around ChaCha20-Poly1305
//! with an HKDF-derived key. Rather than expose the wrappers, we benchmark the
//! primitives directly — the wrapper overhead is dominated by the AEAD.

use chacha20poly1305::{
    aead::{Aead, OsRng},
    AeadCore, ChaCha20Poly1305, KeyInit,
};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use hkdf::Hkdf;
use sha2::Sha256;

fn bench_hkdf_derive(c: &mut Criterion) {
    c.bench_function("crypto/hkdf_sha256_derive", |b| {
        let salt = b"tangle-sandbox-blueprint-paseto-v4";
        let info = b"secrets-at-rest-encryption-v1";
        let ikm = b"some-session-auth-secret-value";
        b.iter(|| {
            let hk = Hkdf::<Sha256>::new(Some(black_box(salt)), black_box(ikm));
            let mut key = [0u8; 32];
            hk.expand(black_box(info), &mut key).expect("hkdf");
            black_box(key);
        })
    });
}

fn bench_chacha_seal_open(c: &mut Criterion) {
    let key_bytes = [0x42u8; 32];
    let cipher = ChaCha20Poly1305::new((&key_bytes).into());

    let mut group = c.benchmark_group("crypto/chacha20poly1305");
    for size in [64usize, 1_024, 16_384, 64_000] {
        let payload = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("seal", size),
            &payload,
            |b, data| {
                b.iter(|| {
                    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
                    let ct = cipher
                        .encrypt(&nonce, black_box(data.as_slice()))
                        .expect("encrypt");
                    black_box(ct);
                })
            },
        );

        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = cipher.encrypt(&nonce, payload.as_slice()).expect("encrypt");
        group.bench_with_input(
            BenchmarkId::new("open", size),
            &(ct, nonce),
            |b, (ct, nonce)| {
                b.iter(|| {
                    let pt = cipher.decrypt(black_box(nonce), ct.as_slice()).expect("decrypt");
                    black_box(pt);
                })
            },
        );
    }
    group.finish();
}

criterion_group!(crypto_benches, bench_hkdf_derive, bench_chacha_seal_open);
criterion_main!(crypto_benches);
