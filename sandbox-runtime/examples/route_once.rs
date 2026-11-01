//! One create through the same `create_sidecar` the on-chain job calls,
//! with `runtime_backend=microsandbox`, then one exec inside the guest
//! and a delete. This is the operator's routing path without a chain.

use sandbox_runtime::runtime::{CreateSandboxParams, create_sidecar, delete_sidecar};

#[tokio::main]
async fn main() {
    for key in [
        "SANDBOX_RUNTIME_BACKEND",
        "SIDECAR_IMAGE",
        "SIDECAR_PUBLIC_HOST",
        "BLUEPRINT_STATE_DIR",
        "SESSION_AUTH_SECRET",
    ] {
        if std::env::var(key).ok().filter(|v| !v.is_empty()).is_none() {
            panic!("{key} must be set before the operator routes a create");
        }
    }

    let mut params = CreateSandboxParams {
        name: "mac-operator-proof".into(),
        image: "alpine:3.20".into(),
        cpu_cores: 1,
        memory_mb: 256,
        metadata_json: r#"{"runtime_backend":"microsandbox"}"#.into(),
        ..CreateSandboxParams::default()
    };
    params.service_id = Some(1);

    let (record, attestation) = create_sidecar(&params, None)
        .await
        .expect("create_sidecar through microsandbox");
    assert!(attestation.is_none());
    println!("sandbox_id={}", record.id);
    println!("container_id={}", record.container_id);
    println!("sidecar_url={}", record.sidecar_url);
    println!("token_len={}", record.token.len());
    println!("metadata={}", record.metadata_json);

    let guest = std::process::Command::new("msb")
        .args(["exec", &record.container_id, "--", "uname", "-a"])
        .output()
        .expect("msb exec");
    println!("guest_uname_status={}", guest.status.code().unwrap_or(-1));
    println!(
        "guest_uname={}",
        String::from_utf8_lossy(&guest.stdout).trim()
    );
    if !guest.stderr.is_empty() {
        eprintln!("guest_stderr={}", String::from_utf8_lossy(&guest.stderr));
    }

    let host = std::process::Command::new("uname")
        .arg("-a")
        .output()
        .expect("host uname");
    println!(
        "host_uname={}",
        String::from_utf8_lossy(&host.stdout).trim()
    );

    delete_sidecar(&record, None)
        .await
        .expect("delete microsandbox");
    let listed = std::process::Command::new("msb")
        .args(["ls"])
        .output()
        .expect("msb ls");
    println!(
        "msb_ls_after_delete:\n{}",
        String::from_utf8_lossy(&listed.stdout)
    );
}
