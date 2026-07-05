use super::*;

/// Build the Docker container config override with port bindings, exposed ports,
/// and resource constraints (CPU, memory).
pub(crate) fn build_docker_config(
    config: &SidecarRuntimeConfig,
    ssh_enabled: bool,
    cpu_cores: u64,
    memory_mb: u64,
    labels: Option<HashMap<String, String>>,
    extra_ports: &[u16],
) -> BollardConfig<String> {
    // Security: ports bound to 127.0.0.1 only — not exposed to external network.
    // Inter-container isolation requires Docker daemon --icc=false configuration.
    let mut port_bindings = PortMap::new();
    port_bindings.insert(
        format!("{}/tcp", config.container_port),
        Some(vec![PortBinding {
            host_ip: Some("127.0.0.1".to_string()),
            host_port: None,
        }]),
    );
    if ssh_enabled {
        port_bindings.insert(
            format!("{}/tcp", config.ssh_port),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: None,
            }]),
        );
    }
    for &port in extra_ports {
        port_bindings.insert(
            format!("{port}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: None,
            }]),
        );
    }

    let mut exposed_ports = HashMap::new();
    exposed_ports.insert(format!("{}/tcp", config.container_port), HashMap::new());
    if ssh_enabled {
        exposed_ports.insert(format!("{}/tcp", config.ssh_port), HashMap::new());
    }
    for &port in extra_ports {
        exposed_ports.insert(format!("{port}/tcp"), HashMap::new());
    }

    // When SIDECAR_NETWORK_HOST=true, use host networking so containers share the
    // host's network namespace. This avoids firewall issues where the host drops
    // traffic from the docker bridge interface. Port bindings are ignored in host
    // network mode — the sidecar binds directly on host ports.
    let use_host_network =
        std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");

    let mut host_config = HostConfig {
        port_bindings: if use_host_network {
            None
        } else {
            Some(port_bindings)
        },
        network_mode: if use_host_network {
            Some("host".to_string())
        } else {
            None
        },
        cap_drop: Some(vec!["ALL".to_string()]),
        cap_add: Some({
            let mut caps = vec![
                "SYS_PTRACE".to_string(),
                "SETGID".to_string(),
                "SETUID".to_string(),
                // Agent frameworks (e.g. opencode) chown workspace dirs on startup.
                "CHOWN".to_string(),
            ];
            if ssh_enabled {
                // OpenSSH's pre-auth sandbox chroots into /var/empty.
                caps.push("SYS_CHROOT".to_string());
                caps.push("NET_BIND_SERVICE".to_string());
                // sshd calls `audit_send_user_message()` on every PTY
                // allocation (interactive shell). Without CAP_AUDIT_WRITE,
                // the audit syscall returns EPERM and `linux_audit_write_entry`
                // fails — modern OpenSSH aborts the session immediately
                // with "Connection closed by remote host" instead of
                // dropping into the shell. Non-interactive (`ssh host cmd`)
                // does not allocate a PTY and works without this cap, which
                // is why command-mode tests pass but `ssh -tt` hangs up.
                caps.push("AUDIT_WRITE".to_string());
                // apt-get install (the openssh-server fallback path for
                // images without a pre-installed sshd) drops fetching to
                // the `_apt` user. With cap_drop=ALL, even in-container
                // root cannot bypass the `_apt`-owned cache directory
                // permissions, so the install fails with
                // `rename failed, Permission denied` and the package
                // lookup degrades to "no installation candidate". Grant
                // DAC_OVERRIDE + FOWNER only for ssh-enabled sandboxes
                // — the widening is scoped to the path that needs it.
                // Long-term, pre-baking openssh-server into the sidecar
                // image lets us drop this back to just the two caps above.
                caps.push("DAC_OVERRIDE".to_string());
                caps.push("FOWNER".to_string());
            }
            caps
        }),
        security_opt: Some(vec!["no-new-privileges=false".to_string()]),
        pids_limit: Some(512),
        readonly_rootfs: Some(false),
        tmpfs: Some(HashMap::from([
            ("/tmp".to_string(), "rw,noexec,nosuid,size=512m".to_string()),
            ("/run".to_string(), "rw,noexec,nosuid,size=64m".to_string()),
        ])),
        // Map host.docker.internal to the host machine so containers can
        // reach host-bound services on the Docker host.
        extra_hosts: if use_host_network {
            None
        } else {
            Some(vec!["host.docker.internal:host-gateway".to_string()])
        },
        ..Default::default()
    };
    if cpu_cores > 0 {
        host_config.nano_cpus = Some((cpu_cores as i64) * 1_000_000_000);
    }
    if memory_mb > 0 {
        host_config.memory = Some((memory_mb as i64) * 1024 * 1024);
    }

    BollardConfig {
        exposed_ports: if use_host_network {
            None
        } else {
            Some(exposed_ports)
        },
        host_config: Some(host_config),
        labels,
        ..Default::default()
    }
}
