//! Micro-benchmarks for the session auth hot path.
//!
//! Every authenticated API request runs through `validate_session_token`, so
//! its latency is a direct component of every API call's tail latency. We
//! measure both the hot path (token found in SESSIONS map) and the cold path
//! (fallback to PASETO decrypt). We also measure the crypto primitives used
//! by the challenge/response flow.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;

use sandbox_runtime::session_auth::{
    clear_all_for_testing, create_challenge, create_test_token, exchange_signature_for_token,
    extract_bearer_token, revoke_session, validate_session_token, verify_eip191_signature,
};

fn sign_eip191(signing_key: &SigningKey, message: &str) -> String {
    use tiny_keccak::{Hasher, Keccak};
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
    let mut digest = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(prefixed.as_bytes());
    hasher.finalize(&mut digest);
    let (sig, rid) = signing_key.sign_prehash_recoverable(&digest).expect("sign");
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&sig.to_bytes());
    bytes.push(rid.to_byte() + 27);
    format!("0x{}", hex::encode(&bytes))
}

fn bench_validate_session_token_hot(c: &mut Criterion) {
    clear_all_for_testing();
    let token = create_test_token("0x1111111111111111111111111111111111111111");
    c.bench_function("session_auth/validate_hot", |b| {
        b.iter(|| black_box(validate_session_token(black_box(&token))))
    });
    clear_all_for_testing();
}

fn bench_validate_session_token_cold(c: &mut Criterion) {
    clear_all_for_testing();
    let token = create_test_token("0x2222222222222222222222222222222222222222");
    clear_all_for_testing();

    c.bench_function("session_auth/validate_cold_paseto_fallback", |b| {
        b.iter(|| black_box(validate_session_token(black_box(&token))))
    });
    clear_all_for_testing();
}

fn bench_validate_session_token_revoked(c: &mut Criterion) {
    clear_all_for_testing();
    let token = create_test_token("0x3333333333333333333333333333333333333333");
    revoke_session(&token);

    c.bench_function("session_auth/validate_revoked_rejected", |b| {
        b.iter(|| {
            let _ = black_box(validate_session_token(black_box(&token)));
        })
    });
    clear_all_for_testing();
}

fn bench_verify_eip191_signature(c: &mut Criterion) {
    let signing_key = SigningKey::random(&mut OsRng);
    let message = "Sign this message to authenticate with Tangle Sandbox.\n\nNonce: deadbeef";
    let sig = sign_eip191(&signing_key, message);

    c.bench_function("session_auth/verify_eip191", |b| {
        b.iter(|| {
            let recovered = verify_eip191_signature(black_box(message), black_box(&sig));
            black_box(recovered.expect("recover"));
        })
    });
}

fn bench_exchange_signature_for_token(c: &mut Criterion) {
    let signing_key = SigningKey::random(&mut OsRng);

    c.bench_function("session_auth/full_challenge_roundtrip", |b| {
        b.iter(|| {
            if rand::random::<u8>() % 64 == 0 {
                clear_all_for_testing();
            }
            let challenge = create_challenge().expect("challenge");
            let sig = sign_eip191(&signing_key, &challenge.message);
            let token = exchange_signature_for_token(&challenge.nonce, &sig).expect("exchange");
            black_box(token);
        })
    });
    clear_all_for_testing();
}

fn bench_extract_bearer_token(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_auth/extract_bearer");
    for (name, header) in [
        ("valid", "Bearer v4.local.abcdefghij"),
        ("missing_scheme", "abcdefghij"),
        ("wrong_scheme", "Basic abcdefghij"),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), header, |b, h| {
            b.iter(|| black_box(extract_bearer_token(black_box(h))));
        });
    }
    group.finish();
}

criterion_group!(
    session_auth_benches,
    bench_validate_session_token_hot,
    bench_validate_session_token_cold,
    bench_validate_session_token_revoked,
    bench_verify_eip191_signature,
    bench_exchange_signature_for_token,
    bench_extract_bearer_token,
);
criterion_main!(session_auth_benches);
