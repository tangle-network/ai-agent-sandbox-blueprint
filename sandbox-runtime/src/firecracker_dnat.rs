//! Per-VM host port forwarding via `iptables` PREROUTING DNAT rules.
//!
//! The Firecracker network primitive [`microvm_runtime::network::NetworkManager`]
//! sets up a host bridge with MASQUERADE NAT, which gives the guest egress.
//! It does *not* set up ingress: host port → guest port forwarding is the
//! caller's job. This module is the caller-side glue.
//!
//! ## Design mirror with `microvm_runtime::firewall`
//!
//! - Per-VM chain in the `nat` table, named
//!   `<chain_prefix><first-16-hex-of-fnv1a-64(vm_id)>` (24 chars, comfortably
//!   under iptables' 28-char cap).
//! - Idempotent `install` — re-installing a different mapping for the same
//!   VM replaces the previous chain.
//! - Idempotent `release` — chains / jumps that are already gone are not an
//!   error.
//! - Every `iptables` invocation passes `-w` so concurrent system tooling
//!   (fail2ban, ufw, …) does not EBUSY us.
//! - All arguments validated before any iptables call so a malformed input
//!   fails fast and visibly instead of producing a half-installed chain.
//!
//! ## Why a separate module from `microvm_runtime::firewall`
//!
//! `microvm_runtime::firewall` ships a FORWARD-chain allowlist (egress
//! scoping). DNAT lives in the `nat` table with completely different
//! semantics — its chain prefix must not collide with the FORWARD one so an
//! orphan-GC of one cannot ever sweep the other. Keeping it sandbox-side
//! also keeps the primitive scope tight; once a generalised DNAT API ships
//! upstream this module can become a thin shim.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, SandboxError};
use crate::runtime::{PortMapping, PortProtocol};

const DEFAULT_IPTABLES_BIN: &str = "iptables";
/// Distinct prefix from `microvm-runtime`'s firewall (`microvm-`) so the two
/// chains can never alias under list-by-prefix queries. Kept short (`mvdnat-`,
/// 7 chars) so the 16-char hash suffix still fits under iptables' 28-char
/// kernel limit on chain names.
const DEFAULT_CHAIN_PREFIX: &str = "mvdnat-";
const CHAIN_HASH_LEN: usize = 16;
/// Iptables IFNAMSIZ-1 cap on interface names. Mirrors the firewall module.
#[cfg(test)]
const IPTABLES_CHAIN_NAME_MAX: usize = 28;

/// FNV-1a 64-bit digest. Dependency-light hashing mirror of
/// `microvm_runtime::firewall`.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn chain_name(vm_id: &str) -> String {
    let digest = fnv1a_64(vm_id.as_bytes());
    let hex = format!("{digest:016x}");
    format!("{DEFAULT_CHAIN_PREFIX}{}", &hex[..CHAIN_HASH_LEN])
}

fn iptables_bin() -> PathBuf {
    std::env::var("MICROVM_IPTABLES_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_IPTABLES_BIN))
}

fn validate_vm_id(vm_id: &str) -> Result<()> {
    if vm_id.is_empty() {
        return Err(SandboxError::Validation(
            "firecracker dnat: vm_id must not be empty".into(),
        ));
    }
    Ok(())
}

fn validate_port(label: &str, port: u16) -> Result<()> {
    if port == 0 {
        return Err(SandboxError::Validation(format!(
            "firecracker dnat: {label} port 0 is reserved"
        )));
    }
    Ok(())
}

fn proto_str(proto: PortProtocol) -> &'static str {
    proto.as_str()
}

/// Install a host → guest port forward. Idempotent: calling twice with the
/// same `vm_id` + different mappings extends the chain. To replace the
/// mapping set, call [`release_port_forwards`] first.
///
/// Internally this:
///
/// 1. Ensures the per-VM chain exists in the `nat` table (`-N <chain>`,
///    suppressing the "already exists" error).
/// 2. Appends a DNAT rule mapping `host_port → guest_ip:container_port`.
/// 3. Inserts a jump from `PREROUTING` to the per-VM chain (idempotent via
///    a `-C` precheck).
///
/// Requires `CAP_NET_ADMIN` on the host. Without it, every call returns
/// [`SandboxError::Unavailable`].
pub(crate) fn install_port_forward(
    vm_id: &str,
    guest_ip: Ipv4Addr,
    mapping: PortMapping,
) -> Result<()> {
    validate_vm_id(vm_id)?;
    validate_port("host_port", mapping.host_port)?;
    validate_port("container_port", mapping.container_port)?;

    let bin = iptables_bin();
    let chain = chain_name(vm_id);
    let proto = proto_str(mapping.protocol);
    let dest = format!("{guest_ip}:{}", mapping.container_port);
    let host_port = mapping.host_port.to_string();

    // 1. Create the per-VM chain if missing.
    match run_iptables(&bin, &["-t", "nat", "-N", &chain]) {
        Ok(_) => {}
        Err(SandboxError::Unavailable(msg)) if is_chain_exists_error(&msg) => {}
        Err(e) => return Err(e),
    }

    // 2. Append the DNAT rule.
    run_iptables(
        &bin,
        &[
            "-t",
            "nat",
            "-A",
            &chain,
            "-p",
            proto,
            "--dport",
            &host_port,
            "-j",
            "DNAT",
            "--to-destination",
            &dest,
        ],
    )?;

    // 3. Jump from PREROUTING into the per-VM chain (idempotent).
    let chain_str = chain.as_str();
    let jump = ["-t", "nat", "PREROUTING", "-p", proto, "-j", chain_str];
    let mut check_args: Vec<&str> = Vec::with_capacity(jump.len() + 1);
    check_args.push("-C");
    check_args.extend_from_slice(&jump);
    if run_iptables(&bin, &check_args).is_err() {
        let mut insert_args: Vec<&str> = Vec::with_capacity(jump.len() + 1);
        insert_args.push("-I");
        insert_args.extend_from_slice(&jump);
        run_iptables(&bin, &insert_args)?;
    }

    Ok(())
}

/// Remove every PREROUTING jump to the per-VM chain, then flush + delete
/// the chain. Idempotent: missing chains / missing jumps are not errors.
pub(crate) fn release_port_forwards(vm_id: &str) -> Result<()> {
    validate_vm_id(vm_id)?;
    let bin = iptables_bin();
    let chain = chain_name(vm_id);

    delete_prerouting_jumps_to(&bin, &chain)?;
    flush_and_delete_chain(&bin, &chain)
}

/// Delete every `nat:PREROUTING` jump targeting `chain`. Mirrors the
/// firewall module's FORWARD-chain GC.
fn delete_prerouting_jumps_to(bin: &Path, chain: &str) -> Result<()> {
    let stdout = iptables_capture(
        bin,
        &["-t", "nat", "-L", "PREROUTING", "--line-numbers", "-n"],
    )?;
    let mut indices: Vec<u32> = Vec::new();
    for line in stdout.lines().skip(2) {
        let mut cols = line.split_whitespace();
        let num = match cols.next().and_then(|n| n.parse::<u32>().ok()) {
            Some(n) => n,
            None => continue,
        };
        let target = match cols.next() {
            Some(t) => t,
            None => continue,
        };
        if target == chain {
            indices.push(num);
        }
    }
    // Delete by descending index so earlier indices stay valid.
    indices.sort_unstable_by(|a, b| b.cmp(a));
    for idx in indices {
        let idx_str = idx.to_string();
        match run_iptables(bin, &["-t", "nat", "-D", "PREROUTING", &idx_str]) {
            Ok(_) => {}
            Err(SandboxError::Unavailable(msg)) if is_not_found_error(&msg) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn flush_and_delete_chain(bin: &Path, chain: &str) -> Result<()> {
    match run_iptables(bin, &["-t", "nat", "-F", chain]) {
        Ok(_) => {}
        Err(SandboxError::Unavailable(msg)) if is_not_found_error(&msg) => return Ok(()),
        Err(e) => return Err(e),
    }
    match run_iptables(bin, &["-t", "nat", "-X", chain]) {
        Ok(_) => Ok(()),
        Err(SandboxError::Unavailable(msg)) if is_not_found_error(&msg) => Ok(()),
        Err(e) => Err(e),
    }
}

fn run_iptables(bin: &Path, args: &[&str]) -> Result<()> {
    let output = spawn_iptables(bin, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SandboxError::Unavailable(format_failure(
            bin, args, &output,
        )))
    }
}

fn iptables_capture(bin: &Path, args: &[&str]) -> Result<String> {
    let output = spawn_iptables(bin, args)?;
    if !output.status.success() {
        return Err(SandboxError::Unavailable(format_failure(
            bin, args, &output,
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn spawn_iptables(bin: &Path, args: &[&str]) -> Result<std::process::Output> {
    let mut cmd = Command::new(bin);
    cmd.arg("-w");
    for a in args {
        cmd.arg(a);
    }
    cmd.output().map_err(|e| {
        SandboxError::Unavailable(format!(
            "firecracker dnat: failed to invoke {} {}: {e}",
            bin.display(),
            args.join(" "),
        ))
    })
}

fn format_failure(bin: &Path, args: &[&str], output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!(
        "firecracker dnat: iptables call failed: {} -w {} (exit={}): stderr={}; stdout={}",
        bin.display(),
        args.join(" "),
        output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr.trim(),
        stdout.trim(),
    )
}

fn is_chain_exists_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("chain already exists") || lower.contains("file exists")
}

fn is_not_found_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("no chain")
        || lower.contains("does not exist")
        || lower.contains("no such")
        || lower.contains("bad rule")
        || lower.contains("matching rule exist")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{PortMapping, PortProtocol};

    #[test]
    fn chain_name_is_deterministic_per_vm() {
        let a = chain_name("vm-alpha");
        let b = chain_name("vm-alpha");
        assert_eq!(a, b);
    }

    #[test]
    fn chain_name_differs_per_vm() {
        assert_ne!(chain_name("vm-a"), chain_name("vm-b"));
    }

    #[test]
    fn chain_name_under_iptables_limit() {
        // Hostile, oversized vm_id still produces a short chain name —
        // pinning the kernel-side iptables limit so a future prefix bump
        // does not silently exceed it.
        let long_id = "this-is-an-arbitrary-firecracker-vm-uuid-1234abcd-5678-deadbeef";
        let name = chain_name(long_id);
        assert!(
            name.len() <= IPTABLES_CHAIN_NAME_MAX,
            "chain name '{name}' exceeds {IPTABLES_CHAIN_NAME_MAX}: {} chars",
            name.len()
        );
        assert!(name.starts_with(DEFAULT_CHAIN_PREFIX));
    }

    #[test]
    fn chain_prefix_does_not_collide_with_firewall_module() {
        // Regression: the egress firewall in `microvm-runtime` uses
        // `microvm-` as its FORWARD-chain prefix. Our DNAT prefix must be
        // distinct so an orphan GC of one cannot sweep the other.
        assert_ne!(DEFAULT_CHAIN_PREFIX, "microvm-");
        assert!(
            !DEFAULT_CHAIN_PREFIX.starts_with("microvm-"),
            "DNAT prefix '{DEFAULT_CHAIN_PREFIX}' must not start with the firewall prefix"
        );
    }

    #[test]
    fn install_validates_vm_id() {
        let err = install_port_forward(
            "",
            Ipv4Addr::new(172, 30, 0, 5),
            PortMapping {
                container_port: 8080,
                host_port: 30000,
                protocol: PortProtocol::Tcp,
            },
        )
        .unwrap_err();
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
    }

    #[test]
    fn install_rejects_zero_host_port() {
        let err = install_port_forward(
            "vm-x",
            Ipv4Addr::new(172, 30, 0, 5),
            PortMapping {
                container_port: 8080,
                host_port: 0,
                protocol: PortProtocol::Tcp,
            },
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
        assert!(msg.contains("host_port"), "{msg}");
    }

    #[test]
    fn install_rejects_zero_container_port() {
        let err = install_port_forward(
            "vm-x",
            Ipv4Addr::new(172, 30, 0, 5),
            PortMapping {
                container_port: 0,
                host_port: 30000,
                protocol: PortProtocol::Tcp,
            },
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
        assert!(msg.contains("container_port"), "{msg}");
    }

    #[test]
    fn release_validates_vm_id() {
        let err = release_port_forwards("").unwrap_err();
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
    }

    #[test]
    fn error_classifier_chain_exists() {
        assert!(is_chain_exists_error("iptables: Chain already exists."));
        assert!(is_chain_exists_error("File exists"));
        assert!(!is_chain_exists_error("Permission denied"));
    }

    #[test]
    fn error_classifier_not_found() {
        assert!(is_not_found_error(
            "iptables: No chain/target/match by that name."
        ));
        assert!(is_not_found_error(
            "iptables: Bad rule (does a matching rule exist in that chain?)"
        ));
        assert!(!is_not_found_error("Permission denied"));
    }

    #[test]
    fn proto_string_mapping() {
        assert_eq!(proto_str(PortProtocol::Tcp), "tcp");
        assert_eq!(proto_str(PortProtocol::Udp), "udp");
    }
}
