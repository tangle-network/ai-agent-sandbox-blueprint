use super::*;

#[cfg(test)]
mod port_mapping_tests {
    use super::*;

    static INIT: std::sync::Once = std::sync::Once::new();

    fn init() {
        INIT.call_once(|| unsafe {
            std::env::set_var("SIDECAR_IMAGE", "test:latest");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        });
    }

    #[test]
    fn parse_ports_from_metadata_json() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [3000, 5432, 9090]}"#, &[]);
        assert_eq!(ports, vec![3000, 5432, 9090]);
    }

    #[test]
    fn parse_ports_from_explicit_field() {
        init();
        let ports = parse_extra_ports("{}", &[3000, 5432]);
        assert_eq!(ports, vec![3000, 5432]);
    }

    #[test]
    fn parse_ports_deduplicates() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [3000, 5432]}"#, &[3000, 9090]);
        assert_eq!(ports, vec![3000, 5432, 9090]);
    }

    #[test]
    fn parse_ports_filters_reserved_sidecar_port() {
        init();
        let config = SidecarRuntimeConfig::load();
        let ports = parse_extra_ports(
            &format!(r#"{{"ports": [{}, 3000]}}"#, config.container_port),
            &[],
        );
        // Sidecar port (8080) should be filtered out
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_filters_reserved_ssh_port() {
        init();
        let config = SidecarRuntimeConfig::load();
        let ports = parse_extra_ports(&format!(r#"{{"ports": [{}, 3000]}}"#, config.ssh_port), &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_filters_zero() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [0, 3000]}"#, &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_caps_at_max() {
        init();
        let all: Vec<u16> = (3000..3020).collect();
        let ports = parse_extra_ports("{}", &all);
        assert_eq!(ports.len(), crate::MAX_EXTRA_PORTS);
    }

    #[test]
    fn parse_ports_empty_metadata() {
        init();
        let ports = parse_extra_ports("{}", &[]);
        assert!(ports.is_empty());
    }

    #[test]
    fn parse_ports_invalid_metadata() {
        init();
        let ports = parse_extra_ports("not-json", &[3000]);
        // Should still parse explicit ports even if metadata is invalid
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_ignores_non_numeric() {
        init();
        let ports = parse_extra_ports(r#"{"ports": ["not-a-port", 3000, true]}"#, &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn build_docker_config_includes_extra_ports() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, false, 1, 512, None, &[3000, 5432]);

        let exposed = docker_config.exposed_ports.unwrap();
        assert!(exposed.contains_key("3000/tcp"));
        assert!(exposed.contains_key("5432/tcp"));
        assert!(exposed.contains_key(&format!("{}/tcp", config.container_port)));

        let bindings = docker_config.host_config.unwrap().port_bindings.unwrap();
        assert!(bindings.contains_key("3000/tcp"));
        assert!(bindings.contains_key("5432/tcp"));
    }

    #[test]
    fn build_docker_config_no_extra_ports() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, false, 1, 512, None, &[]);

        let exposed = docker_config.exposed_ports.unwrap();
        // Only sidecar port should be exposed (no SSH since ssh_enabled=false)
        assert_eq!(exposed.len(), 1);
        assert!(exposed.contains_key(&format!("{}/tcp", config.container_port)));
    }

    #[test]
    fn build_docker_config_adds_ssh_caps_when_enabled() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, true, 1, 512, None, &[]);

        let caps = docker_config.host_config.unwrap().cap_add.unwrap();
        assert!(caps.contains(&"CHOWN".to_string()));
        assert!(caps.contains(&"NET_BIND_SERVICE".to_string()));
        assert!(caps.contains(&"SYS_CHROOT".to_string()));
        // DAC_OVERRIDE + FOWNER are required for `apt-get install
        // openssh-server` to succeed in images without a pre-baked sshd —
        // apt drops to `_apt` for fetching and in-container root cannot
        // bypass the `_apt`-owned partial cache without DAC_OVERRIDE.
        // Regression for the ssh_e2e flake where install failed with
        // `rename failed, Permission denied`.
        assert!(
            caps.contains(&"DAC_OVERRIDE".to_string()),
            "DAC_OVERRIDE must be granted when ssh_enabled so apt can install openssh-server"
        );
        assert!(
            caps.contains(&"FOWNER".to_string()),
            "FOWNER must be granted when ssh_enabled so apt can manage _apt-owned files"
        );
        // AUDIT_WRITE is required for sshd's PTY allocation path. Without
        // it interactive shells (`ssh -tt`) fail at session start with
        // "Connection closed by remote host" while non-interactive command
        // mode (`ssh host cmd`) still works. Regression for the
        // ssh_e2e interactive-shell assertion.
        assert!(
            caps.contains(&"AUDIT_WRITE".to_string()),
            "AUDIT_WRITE must be granted when ssh_enabled so sshd can allocate PTYs"
        );
    }

    /// SSH-specific capability widening must NOT leak into non-SSH sandboxes.
    /// The widening is justified by the apt fallback path only, and the
    /// security-minimal default profile (no DAC_OVERRIDE) must hold for
    /// every other configuration.
    #[test]
    fn build_docker_config_omits_ssh_caps_when_disabled() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, false, 1, 512, None, &[]);

        let caps = docker_config.host_config.unwrap().cap_add.unwrap();
        assert!(!caps.contains(&"DAC_OVERRIDE".to_string()));
        assert!(!caps.contains(&"FOWNER".to_string()));
        assert!(!caps.contains(&"AUDIT_WRITE".to_string()));
        assert!(!caps.contains(&"SYS_CHROOT".to_string()));
        assert!(!caps.contains(&"NET_BIND_SERVICE".to_string()));
    }

    #[test]
    fn docker_ssh_bootstrap_unlocks_login_user() {
        let command = build_docker_ssh_bootstrap_command("agent");
        // `passwd -d` is the primary unlock — it removes the password and
        // clears the lock flag (`NP` in passwd -S), the right state for
        // key-only login. `passwd -u` is the secondary because it fails on
        // passwordless accounts (the common case for `useradd`-created
        // service users), which leaves the shadow entry `!`-prefixed and
        // modern OpenSSH rejects auth before checking authorized_keys.
        assert!(command.contains("passwd -d \"$user\""));
        assert!(command.contains("passwd -u \"$user\""));
        assert!(command.contains("AllowUsers agent"));
        assert!(!command.contains("pipefail"));
    }

    /// `apt-get install` exits non-zero on this image family when its
    /// partial-cache cleanup hits `_apt`-owned files it can't remove
    /// (rootless / user-namespace-remapped Docker), even though the package
    /// itself installed cleanly. The bootstrap must not abort on that
    /// best-effort cleanup failure — it must check `command -v sshd` as
    /// the actual success criterion. Regression for the
    /// `docker_ssh_supports_commands_and_interactive_shell` flake where
    /// the stderr `rm: cannot remove '/var/cache/apt/archives/partial/*.deb'`
    /// fell through `set -e` and failed the bootstrap.
    #[test]
    fn docker_ssh_bootstrap_tolerates_apt_cleanup_failure() {
        let command = build_docker_ssh_bootstrap_command("agent");
        // Both the install and the lists-cleanup must be wrapped with
        // `|| true` so a benign non-zero exit doesn't trip `set -e`.
        assert!(
            command.contains(
                "apt-get install -y --no-install-recommends openssh-server >/dev/null 2>&1 || true"
            ),
            "apt-get install must tolerate cache-cleanup failures"
        );
        assert!(
            command.contains("rm -rf /var/lib/apt/lists/* 2>/dev/null || true"),
            "apt lists cleanup must be best-effort"
        );
        // After the tolerant install, the bootstrap must hard-fail when
        // sshd is genuinely missing — otherwise we'd silently start sshd
        // setup against a container with no sshd binary.
        assert!(
            command.contains("openssh-server install failed: sshd binary missing"),
            "bootstrap must verify sshd actually installed before continuing"
        );
    }

    #[test]
    fn select_docker_ssh_login_user_prefers_sidecar_then_agent() {
        let selected = select_docker_ssh_login_user(|candidate| candidate == "agent");
        assert_eq!(selected, Some("agent"));

        let selected = select_docker_ssh_login_user(|candidate| {
            candidate == SSH_DEFAULT_LOGIN_USER || candidate == SSH_FALLBACK_LOGIN_USER
        });
        assert_eq!(selected, Some(SSH_DEFAULT_LOGIN_USER));
    }

    #[test]
    fn select_docker_ssh_login_user_returns_none_when_no_compatible_user_exists() {
        let selected = select_docker_ssh_login_user(|_| false);
        assert_eq!(selected, None);
    }

    #[test]
    fn extra_ports_serde_roundtrip() {
        let mut ports = HashMap::new();
        ports.insert(3000u16, 32768u16);
        ports.insert(5432, 32769);

        let json = serde_json::to_string(&ports).unwrap();
        let restored: HashMap<u16, u16> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.get(&3000), Some(&32768));
        assert_eq!(restored.get(&5432), Some(&32769));
    }

    #[test]
    fn extra_ports_default_empty_on_deserialize() {
        // Simulates loading a record from before extra_ports existed
        let json = r#"{"id":"test","container_id":"c","sidecar_url":"http://x","sidecar_port":0,"token":"t","created_at":0}"#;
        let record: SandboxRecord = serde_json::from_str(json).unwrap();
        assert!(record.extra_ports.is_empty());
    }
}

#[cfg(test)]
mod metadata_port_mapping_tests {
    use super::*;

    fn meta(s: &str) -> Value {
        serde_json::from_str(s).expect("test metadata is valid JSON")
    }

    #[test]
    fn parse_metadata_ports_absent_field_returns_empty() {
        let m = meta(r#"{}"#);
        assert!(parse_metadata_ports(&m).unwrap().is_empty());
    }

    #[test]
    fn parse_metadata_ports_null_field_returns_empty() {
        let m = meta(r#"{"ports": null}"#);
        assert!(parse_metadata_ports(&m).unwrap().is_empty());
    }

    #[test]
    fn parse_metadata_ports_empty_array_returns_empty() {
        let m = meta(r#"{"ports": []}"#);
        assert!(parse_metadata_ports(&m).unwrap().is_empty());
    }

    #[test]
    fn parse_metadata_ports_legacy_bare_integer() {
        let m = meta(r#"{"ports": [3000]}"#);
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].container_port, 3000);
        assert_eq!(parsed[0].host_port, 3000);
        assert_eq!(parsed[0].protocol, PortProtocol::Tcp);
    }

    #[test]
    fn parse_metadata_ports_single_structured_mapping() {
        let m =
            meta(r#"{"ports": [{"container_port": 3000, "host_port": 30000, "protocol": "tcp"}]}"#);
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].container_port, 3000);
        assert_eq!(parsed[0].host_port, 30000);
        assert_eq!(parsed[0].protocol, PortProtocol::Tcp);
    }

    #[test]
    fn parse_metadata_ports_multiple_structured_mappings() {
        let m = meta(
            r#"{"ports": [
                {"container_port": 3000, "host_port": 30000, "protocol": "tcp"},
                {"container_port": 5432, "host_port": 30001, "protocol": "tcp"},
                {"container_port": 53,   "host_port": 30053, "protocol": "udp"}
            ]}"#,
        );
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[2].protocol, PortProtocol::Udp);
        assert_eq!(parsed[2].host_port, 30053);
    }

    #[test]
    fn parse_metadata_ports_protocol_defaults_to_tcp() {
        let m = meta(r#"{"ports": [{"container_port": 3000, "host_port": 30000}]}"#);
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed[0].protocol, PortProtocol::Tcp);
    }

    #[test]
    fn parse_metadata_ports_protocol_case_insensitive() {
        let m =
            meta(r#"{"ports": [{"container_port": 3000, "host_port": 30000, "protocol": "TCP"}]}"#);
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed[0].protocol, PortProtocol::Tcp);
    }

    #[test]
    fn parse_metadata_ports_rejects_port_out_of_range_legacy() {
        let m = meta(r#"{"ports": [70000]}"#);
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("out of range"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_rejects_port_out_of_range_structured() {
        let m = meta(
            r#"{"ports": [{"container_port": 70000, "host_port": 30000, "protocol": "tcp"}]}"#,
        );
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("out of range"), "{msg}");
        assert!(msg.contains("container_port"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_rejects_port_zero() {
        let m = meta(r#"{"ports": [0]}"#);
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("0 is reserved") || msg.contains("reserved"),
            "{msg}"
        );
    }

    #[test]
    fn parse_metadata_ports_rejects_unknown_protocol() {
        let m = meta(
            r#"{"ports": [{"container_port": 3000, "host_port": 30000, "protocol": "sctp"}]}"#,
        );
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("tcp") || msg.contains("udp"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_rejects_duplicate_host_port() {
        let m = meta(
            r#"{"ports": [
                {"container_port": 3000, "host_port": 30000, "protocol": "tcp"},
                {"container_port": 3001, "host_port": 30000, "protocol": "tcp"}
            ]}"#,
        );
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate host_port"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_allows_same_host_port_on_different_protocols() {
        // tcp/30000 and udp/30000 are distinct sockets and must both be allowed.
        let m = meta(
            r#"{"ports": [
                {"container_port": 53, "host_port": 30053, "protocol": "tcp"},
                {"container_port": 53, "host_port": 30053, "protocol": "udp"}
            ]}"#,
        );
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn parse_metadata_ports_rejects_duplicate_container_port_same_protocol() {
        let m = meta(
            r#"{"ports": [
                {"container_port": 3000, "host_port": 30000, "protocol": "tcp"},
                {"container_port": 3000, "host_port": 30001, "protocol": "tcp"}
            ]}"#,
        );
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate container_port"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_rejects_non_array_field() {
        let m = meta(r#"{"ports": 3000}"#);
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be an array"), "{msg}");
    }

    #[test]
    fn parse_metadata_ports_caps_at_max_extra_ports() {
        // 12 entries → output capped to MAX_EXTRA_PORTS.
        let mut entries = String::from("[");
        for i in 0..12 {
            if i > 0 {
                entries.push(',');
            }
            entries.push_str(&format!(
                r#"{{"container_port": {}, "host_port": {}, "protocol": "tcp"}}"#,
                3000 + i,
                30000 + i,
            ));
        }
        entries.push(']');
        let m = meta(&format!(r#"{{"ports": {entries}}}"#));
        let parsed = parse_metadata_ports(&m).unwrap();
        assert_eq!(parsed.len(), crate::MAX_EXTRA_PORTS);
    }

    #[test]
    fn parse_metadata_ports_rejects_missing_field() {
        let m = meta(r#"{"ports": [{"container_port": 3000, "protocol": "tcp"}]}"#);
        let err = parse_metadata_ports(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing field host_port"), "{msg}");
    }
}

#[cfg(test)]
mod sidecar_capability_tests {
    use super::*;

    #[test]
    fn parse_sidecar_capabilities_handles_json_array() {
        assert_eq!(
            parse_sidecar_capabilities(r#"["computer_use"]"#).as_deref(),
            Some("computer_use"),
        );
        assert_eq!(
            parse_sidecar_capabilities(r#"["all_harness"]"#).as_deref(),
            Some("all_harness"),
        );
        assert_eq!(
            parse_sidecar_capabilities(r#"["computer_use","all_harness"]"#).as_deref(),
            Some("computer_use,all_harness"),
        );
    }

    #[test]
    fn parse_sidecar_capabilities_handles_comma_list() {
        assert_eq!(
            parse_sidecar_capabilities("computer_use").as_deref(),
            Some("computer_use"),
        );
        // Tolerate whitespace.
        assert_eq!(
            parse_sidecar_capabilities("  computer_use  ").as_deref(),
            Some("computer_use"),
        );
    }

    #[test]
    fn parse_sidecar_capabilities_drops_unknown_silently() {
        // Forward-compat: a future SDK that names a cap this orchestrator
        // does not yet know must not crash the create — it just won't get
        // the unrecognized subsystem.
        assert_eq!(
            parse_sidecar_capabilities(r#"["computer_use","future_cap"]"#).as_deref(),
            Some("computer_use"),
        );
        assert!(parse_sidecar_capabilities("future_cap").is_none());
    }

    #[test]
    fn parse_sidecar_capabilities_handles_empty_or_malformed() {
        assert!(parse_sidecar_capabilities("").is_none());
        assert!(parse_sidecar_capabilities("   ").is_none());
        assert!(parse_sidecar_capabilities("[]").is_none());
        assert!(parse_sidecar_capabilities("[not json").is_none());
    }

    #[test]
    fn build_env_vars_injects_sidecar_capabilities_for_docker() {
        // Regression: the Docker runtime path must put SIDECAR_CAPABILITIES
        // on the container env so the sidecar boots Xvfb. The capability
        // contract is identical across Docker / Firecracker / TEE; this
        // pins the Docker side. (TEE is covered by tee_deploy_params_*
        // in tee/mod.rs and Firecracker is exercised by integration.)
        let env_vars = build_env_vars("{}", "tok", 8080, r#"["computer_use"]"#).unwrap();
        assert!(
            env_vars.contains(&"SIDECAR_CAPABILITIES=computer_use".to_string()),
            "expected SIDECAR_CAPABILITIES in env, got {env_vars:?}",
        );
        let env_vars =
            build_env_vars("{}", "tok", 8080, r#"["computer_use","all_harness"]"#).unwrap();
        assert!(
            env_vars.contains(&"SIDECAR_CAPABILITIES=computer_use,all_harness".to_string()),
            "expected all-harness SIDECAR_CAPABILITIES in env, got {env_vars:?}",
        );
    }

    #[test]
    fn build_env_vars_omits_capabilities_when_unset() {
        let env_vars = build_env_vars("{}", "tok", 8080, "").unwrap();
        assert!(
            !env_vars
                .iter()
                .any(|v| v.starts_with("SIDECAR_CAPABILITIES=")),
            "expected no SIDECAR_CAPABILITIES env var, got {env_vars:?}",
        );
    }
}

#[cfg(test)]
mod runtime_backend_tests {
    use super::*;

    fn params(metadata_json: &str) -> CreateSandboxParams {
        CreateSandboxParams {
            metadata_json: metadata_json.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn parse_runtime_backend_aliases() {
        assert_eq!(
            parse_runtime_backend_value("docker"),
            Some(RuntimeBackend::Docker)
        );
        assert_eq!(
            parse_runtime_backend_value("container"),
            Some(RuntimeBackend::Docker)
        );
        assert_eq!(
            parse_runtime_backend_value("firecracker"),
            Some(RuntimeBackend::Firecracker)
        );
        assert_eq!(
            parse_runtime_backend_value("microvm"),
            Some(RuntimeBackend::Firecracker)
        );
        assert_eq!(
            parse_runtime_backend_value("tee"),
            Some(RuntimeBackend::Tee)
        );
        assert_eq!(
            parse_runtime_backend_value("confidential-vm"),
            Some(RuntimeBackend::Tee)
        );
        assert_eq!(parse_runtime_backend_value("unknown"), None);
    }

    #[test]
    fn resolve_runtime_backend_from_metadata() {
        let resolved = resolve_runtime_backend(&params(r#"{"runtime_backend":"firecracker"}"#));
        assert_eq!(resolved.unwrap(), RuntimeBackend::Firecracker);

        let resolved_nested =
            resolve_runtime_backend(&params(r#"{"runtime":{"backend":"tee"}}"#)).unwrap();
        assert_eq!(resolved_nested, RuntimeBackend::Tee);
    }

    #[test]
    fn resolve_runtime_backend_forces_tee_when_required() {
        let mut request = params(r#"{"runtime_backend":"docker"}"#);
        request.tee_config = Some(crate::tee::TeeConfig {
            required: true,
            tee_type: crate::tee::TeeType::Tdx,
            attestation_nonce: None,
        });
        let resolved = resolve_runtime_backend(&request).unwrap();
        assert_eq!(resolved, RuntimeBackend::Tee);
    }

    #[test]
    fn resolve_runtime_backend_rejects_firecracker_plus_tee_required() {
        let mut request = params(r#"{"runtime_backend":"firecracker"}"#);
        request.tee_config = Some(crate::tee::TeeConfig {
            required: true,
            tee_type: crate::tee::TeeType::Tdx,
            attestation_nonce: None,
        });
        let err = resolve_runtime_backend(&request).unwrap_err().to_string();
        assert!(err.contains("incompatible"));
    }
}

#[cfg(test)]
mod tee_tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init() {
        INIT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("runtime-tee-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe {
                std::env::set_var("BLUEPRINT_STATE_DIR", dir.to_str().unwrap());
                std::env::set_var("SIDECAR_IMAGE", "test:latest");
                std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            }
        });
    }

    fn tee_required_params() -> CreateSandboxParams {
        CreateSandboxParams {
            name: "tee-test".into(),
            image: "test:latest".into(),
            tee_config: Some(crate::tee::TeeConfig {
                required: true,
                tee_type: crate::tee::TeeType::Tdx,
                attestation_nonce: None,
            }),
            owner: "0xabcdef".into(),
            cpu_cores: 2,
            memory_mb: 4096,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_sidecar_tee_required_no_backend() {
        init();
        let params = tee_required_params();
        let result = create_sidecar(&params, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("TEE runtime selected but no TEE backend configured"),
            "unexpected: {err}"
        );
    }

    #[tokio::test]
    async fn create_sidecar_tee_success() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);
        let params = tee_required_params();

        let (record, attestation) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Record should have TEE fields
        assert!(record.tee_deployment_id.is_some());
        assert!(record.container_id.starts_with("tee-"));
        assert!(record.sidecar_url.starts_with("http://mock-tee:"));
        assert!(record.tee_metadata_json.is_some());
        assert!(record.tee_config.is_some());
        assert_eq!(record.owner, "0xabcdef");
        assert_eq!(record.cpu_cores, 2);
        assert_eq!(record.memory_mb, 4096);

        // Attestation should be present
        let att = attestation.unwrap();
        assert_eq!(att.tee_type, crate::tee::TeeType::Tdx);

        // Mock should have been called
        assert_eq!(
            mock.deploy_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn create_sidecar_tee_stores_record() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Nitro);
        let mut params = tee_required_params();
        params.tee_config.as_mut().unwrap().tee_type = crate::tee::TeeType::None;

        let (record, _) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Verify the record is in the store
        let stored = sandboxes().unwrap().get(&record.id).unwrap().unwrap();
        assert_eq!(stored.id, record.id);
        assert_eq!(stored.tee_deployment_id, record.tee_deployment_id);
        assert!(stored.container_id.starts_with("tee-"));
    }

    #[tokio::test]
    async fn create_sidecar_tee_deploy_failure() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::failing(crate::tee::TeeType::Tdx);
        let params = tee_required_params();

        let result = create_sidecar(&params, Some(&mock)).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Mock deploy failure")
        );
    }

    #[tokio::test]
    async fn delete_sidecar_tee_calls_destroy() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);

        // First create a TEE sandbox
        let params = tee_required_params();
        let (record, _) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Now delete it
        delete_sidecar(&record, Some(&mock)).await.unwrap();
        assert_eq!(
            mock.destroy_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn create_sidecar_non_tee_skips_mock() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);
        let params = CreateSandboxParams {
            name: "docker-test".into(),
            image: "test:latest".into(),
            tee_config: None, // no TEE
            ..Default::default()
        };

        // This will try Docker (and fail since no Docker in tests), but
        // the mock's deploy should NOT be called.
        let _ = create_sidecar(&params, Some(&mock)).await;
        assert_eq!(
            mock.deploy_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "Mock deploy should not be called for non-TEE requests"
        );
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn seal_unseal_roundtrip() {
        let plaintext = "super-secret-token-123";
        let sealed = seal_field(plaintext).unwrap();
        assert!(sealed.starts_with(ENC_PREFIX), "should have enc prefix");
        assert_ne!(sealed, plaintext);

        let unsealed = unseal_field(&sealed).unwrap();
        assert_eq!(unsealed, plaintext);
    }

    #[test]
    fn unseal_plaintext_passthrough() {
        let plain = "not-encrypted-token";
        let result = unseal_field(plain).unwrap();
        assert_eq!(result, plain, "plaintext should pass through unchanged");
    }

    #[test]
    fn seal_empty_string() {
        let sealed = seal_field("").unwrap();
        assert_eq!(sealed, "", "empty string should stay empty");
        let unsealed = unseal_field("").unwrap();
        assert_eq!(unsealed, "", "empty unseal should stay empty");
    }

    #[test]
    fn seal_record_roundtrip() {
        let mut record = SandboxRecord {
            id: "test".into(),
            container_id: "ctr".into(),
            sidecar_url: "http://x".into(),
            sidecar_port: 0,
            ssh_port: None,
            token: "my-token".into(),
            created_at: 0,
            cpu_cores: 0,
            memory_mb: 0,
            state: SandboxState::Running,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 0,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: String::new(),
            base_env_json: r#"{"KEY":"val"}"#.into(),
            user_env_json: r#"{"USER":"x"}"#.into(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
            service_id: None,
            tee_config: None,
            extra_ports: HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
            capabilities_json: String::new(),
        };

        seal_record(&mut record).unwrap();
        assert!(record.token.starts_with(ENC_PREFIX));
        assert!(record.base_env_json.starts_with(ENC_PREFIX));
        assert!(record.user_env_json.starts_with(ENC_PREFIX));

        unseal_record(&mut record).unwrap();
        assert_eq!(record.token, "my-token");
        assert_eq!(record.base_env_json, r#"{"KEY":"val"}"#);
        assert_eq!(record.user_env_json, r#"{"USER":"x"}"#);
    }

    #[test]
    fn unseal_corrupted_ciphertext_returns_error() {
        // Valid prefix but garbage base64 payload (nonce + corrupted ciphertext)
        let corrupted = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD
                .encode(b"123456789012XXXX_corrupted_data_here")
        );
        let result = unseal_field(&corrupted);
        assert!(result.is_err(), "corrupted ciphertext should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("decrypt failed"),
            "error should mention decrypt failure: {err_msg}"
        );
    }

    #[test]
    fn seal_produces_different_ciphertext_each_time() {
        let plaintext = "determinism-test";
        let sealed1 = seal_field(plaintext).unwrap();
        let sealed2 = seal_field(plaintext).unwrap();
        assert_ne!(
            sealed1, sealed2,
            "each seal call should use a random nonce, producing different output"
        );

        // Both should decrypt back to the same plaintext
        assert_eq!(unseal_field(&sealed1).unwrap(), plaintext);
        assert_eq!(unseal_field(&sealed2).unwrap(), plaintext);
    }

    #[test]
    fn unseal_short_ciphertext_returns_error() {
        // Prefix present but payload too short to contain a 12-byte nonce
        let short = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(b"short")
        );
        let result = unseal_field(&short);
        assert!(result.is_err(), "too-short ciphertext should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("too short"),
            "error should mention 'too short': {err_msg}"
        );
    }

    #[test]
    fn unseal_invalid_base64_returns_error() {
        let bad = format!("{ENC_PREFIX}!!!not-valid-base64!!!");
        let result = unseal_field(&bad);
        assert!(result.is_err(), "invalid base64 should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("base64"),
            "error should mention base64: {err_msg}"
        );
    }

    #[test]
    fn seal_large_value() {
        // 1 MB plaintext — verifies no size-related panics or truncation
        let plaintext = "A".repeat(1024 * 1024);
        let sealed = seal_field(&plaintext).unwrap();
        assert!(sealed.starts_with(ENC_PREFIX), "should have enc prefix");
        // Ciphertext + nonce + base64 overhead makes it larger
        assert!(
            sealed.len() > plaintext.len(),
            "sealed form should be larger than plaintext"
        );

        let unsealed = unseal_field(&sealed).unwrap();
        assert_eq!(
            unsealed.len(),
            plaintext.len(),
            "unsealed length should match original"
        );
        assert_eq!(unsealed, plaintext, "unsealed value should match original");
    }

    #[test]
    fn unseal_tampered_ciphertext() {
        // Seal a real value, then flip a byte in the ciphertext portion
        let plaintext = "sensitive-data-that-must-not-silently-corrupt";
        let sealed = seal_field(plaintext).unwrap();

        // Decode, tamper, re-encode
        let encoded = &sealed[ENC_PREFIX.len()..];
        let mut blob = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        // Flip a byte in the ciphertext portion (past the 12-byte nonce)
        assert!(blob.len() > 13, "blob should be longer than nonce");
        blob[13] ^= 0xFF;
        let tampered = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(&blob)
        );

        let result = unseal_field(&tampered);
        assert!(
            result.is_err(),
            "tampered ciphertext must fail authentication, not return corrupted data"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("decrypt failed"),
            "error should mention decrypt failure: {err_msg}"
        );
    }
}

#[cfg(test)]
mod core_logic_tests {
    use super::*;
    use docktopus::bollard::models::{ContainerInspectResponse, NetworkSettings, PortBinding};

    // ── effective_idle_timeout ───────────────────────────────────────────

    fn test_config() -> SidecarRuntimeConfig {
        SidecarRuntimeConfig {
            image: "test".into(),
            public_host: "127.0.0.1".into(),
            container_port: 3000,
            ssh_port: 2222,
            timeout: Duration::from_secs(30),
            docker_host: None,
            pull_image: false,
            sandbox_default_idle_timeout: 1800,
            sandbox_default_max_lifetime: 86400,
            sandbox_max_idle_timeout: 7200,
            sandbox_max_max_lifetime: 172800,
            sandbox_reaper_interval: 30,
            sandbox_gc_interval: 3600,
            sandbox_gc_hot_retention: 86400,
            sandbox_gc_warm_retention: 172800,
            sandbox_gc_cold_retention: 604800,
            snapshot_auto_commit: true,
            snapshot_destination_prefix: None,
            sandbox_max_count: 100,
            sandbox_max_cpu_cores: 0,
            sandbox_max_memory_mb: 0,
            sandbox_max_disk_gb: 0,
            sandbox_host_memory_budget_mb: 0,
        }
    }

    #[test]
    fn adjusted_sandbox_count_reuses_existing_slot() {
        assert_eq!(adjusted_sandbox_count_for_limit(0, false), 0);
        assert_eq!(adjusted_sandbox_count_for_limit(1, false), 1);
        assert_eq!(adjusted_sandbox_count_for_limit(1, true), 0);
        assert_eq!(adjusted_sandbox_count_for_limit(5, true), 4);
    }

    // ── admission control: count-limit class, resource maxima, memory budget ──

    #[test]
    fn count_limit_rejection_is_unavailable() {
        // Capacity exhaustion must map to Unavailable (→ 503) so callers
        // retry on another operator; Validation (→ 400) would tell them the
        // request itself is malformed.
        let err = check_sandbox_count_limit(3, false, 3).unwrap_err();
        assert!(matches!(err, SandboxError::Unavailable(_)), "got {err:?}");
        assert!(err.to_string().contains("Sandbox limit reached (3/3)"));
    }

    #[test]
    fn count_limit_uncapped_reuse_and_in_range_pass() {
        assert!(
            check_sandbox_count_limit(10_000, false, 0).is_ok(),
            "0 = no cap"
        );
        assert!(
            check_sandbox_count_limit(3, true, 3).is_ok(),
            "replacing an existing slot stays within the cap"
        );
        assert!(check_sandbox_count_limit(2, false, 3).is_ok());
    }

    #[test]
    fn resource_max_uncapped_passthrough() {
        assert_eq!(enforce_resource_max(0, 0, "memory_mb").unwrap(), 0);
        assert_eq!(enforce_resource_max(4096, 0, "memory_mb").unwrap(), 4096);
    }

    #[test]
    fn resource_max_clamps_unlimited_request_to_max() {
        // 0 = unlimited must clamp to the cap: an operator who sets a maximum
        // must never run an unlimited container.
        assert_eq!(enforce_resource_max(0, 2048, "memory_mb").unwrap(), 2048);
    }

    #[test]
    fn resource_max_rejects_over_max_as_unavailable() {
        let err = enforce_resource_max(4096, 2048, "memory_mb").unwrap_err();
        assert!(matches!(err, SandboxError::Unavailable(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(
            msg.contains("memory_mb"),
            "message names the resource: {msg}"
        );
        assert!(
            msg.contains("4096") && msg.contains("2048"),
            "message names both values: {msg}"
        );
    }

    #[test]
    fn resource_max_in_range_passthrough() {
        assert_eq!(enforce_resource_max(1024, 2048, "memory_mb").unwrap(), 1024);
    }

    #[test]
    fn accounted_memory_prefers_request_then_max_then_unknown() {
        assert_eq!(accounted_memory_mb(1024, 2048), Some(1024));
        assert_eq!(accounted_memory_mb(0, 2048), Some(2048));
        assert_eq!(accounted_memory_mb(0, 0), None);
    }

    #[test]
    fn memory_budget_disabled_when_zero() {
        assert!(check_host_memory_budget([u64::MAX, u64::MAX], 999_999, 0, 0, 0).is_ok());
    }

    #[test]
    fn memory_budget_rejects_over_budget_as_unavailable() {
        // 1024 + 2048 running + 2048 requested = 5120 > 4096.
        let err = check_host_memory_budget([1024, 2048], 2048, 2048, 4096, 0).unwrap_err();
        assert!(matches!(err, SandboxError::Unavailable(_)), "got {err:?}");
        assert!(err.to_string().contains("memory budget"), "got {err}");
    }

    #[test]
    fn memory_budget_admits_exactly_at_budget() {
        assert!(check_host_memory_budget([1024, 2048], 1024, 2048, 4096, 0).is_ok());
    }

    #[test]
    fn memory_budget_counts_unlimited_records_at_max() {
        // A running record with memory_mb=0 is accounted at SANDBOX_MAX_MEMORY_MB:
        // 2048 (0→max) + 2048 requested = 4096 ≤ 4096 passes…
        assert!(check_host_memory_budget([0], 2048, 2048, 4096, 0).is_ok());
        // …and one extra MB of running memory rejects.
        assert!(check_host_memory_budget([0, 1], 2048, 2048, 4096, 0).is_err());
    }

    #[test]
    fn memory_budget_skips_unaccountable_records() {
        // Without SANDBOX_MAX_MEMORY_MB, unlimited records can't be accounted
        // and are skipped (warned once) rather than guessed.
        assert!(check_host_memory_budget([0, 0], 1024, 0, 2048, 0).is_ok());
        assert!(check_host_memory_budget([1500, 0], 1024, 0, 2048, 0).is_err());
    }

    #[test]
    fn memory_budget_reserves_warm_pool_footprint() {
        // The warm-pool reservation counts toward committed memory: 2048 MB
        // reserved + 1 MB running exceeds a 2048 MB budget.
        assert!(check_host_memory_budget([1], 0, 2048, 2048, 2048).is_err());
        // Reservation exactly at budget, nothing else running, admits.
        assert!(check_host_memory_budget([0u64; 0], 0, 0, 2048, 2048).is_ok());
        // One incoming MB over the reservation rejects.
        assert!(check_host_memory_budget([0u64; 0], 1, 2048, 2048, 2048).is_err());
        // A zero budget still disables the check even with a reservation set.
        assert!(check_host_memory_budget([0u64; 0], 0, 0, 0, 4096).is_ok());
    }

    #[test]
    fn effective_idle_timeout_zero_and_clamped() {
        let cfg = test_config();
        assert_eq!(cfg.effective_idle_timeout(0), 1800, "zero → default");
        assert_eq!(
            cfg.effective_idle_timeout(99999),
            7200,
            "over max → clamped"
        );
        assert_eq!(
            cfg.effective_idle_timeout(3600),
            3600,
            "in range → passthrough"
        );
    }

    #[test]
    fn effective_max_lifetime_zero_and_clamped() {
        let cfg = test_config();
        assert_eq!(cfg.effective_max_lifetime(0), 86400, "zero → default");
        assert_eq!(
            cfg.effective_max_lifetime(999999),
            172800,
            "over max → clamped"
        );
        assert_eq!(
            cfg.effective_max_lifetime(100000),
            100000,
            "in range → passthrough"
        );
    }

    // ── build_env_vars ──────────────────────────────────────────────────

    #[test]
    fn env_vars_with_json() {
        let vars =
            build_env_vars(r#"{"API_KEY":"sk-test","DEBUG":"true"}"#, "tok", 8080, "").unwrap();
        assert!(vars.contains(&"API_KEY=sk-test".to_string()));
        assert!(vars.contains(&"DEBUG=true".to_string()));
        assert!(vars.contains(&"SIDECAR_PORT=8080".to_string()));
    }

    #[test]
    fn env_vars_invalid_json() {
        let result = build_env_vars("not-json", "tok", 3000, "");
        assert!(result.is_err());
    }

    #[test]
    fn env_vars_preserve_explicit_ai_env() {
        let vars = build_env_vars(r#"{"ZAI_API_KEY":"user-key"}"#, "tok", 8080, "").unwrap();
        assert!(vars.contains(&"ZAI_API_KEY=user-key".to_string()));
        assert!(!vars.contains(&"OPENCODE_MODEL_API_KEY=user-key".to_string()));
    }

    #[test]
    fn workflow_runtime_credentials_available_requires_sandbox_env() {
        assert!(!workflow_runtime_credentials_available("{}").unwrap());
    }

    #[test]
    fn workflow_runtime_credentials_available_rejects_incomplete_explicit_ai_env() {
        let old = std::env::var("ZAI_API_KEY").ok();
        // SAFETY: test scopes environment mutation and restores the prior value.
        unsafe {
            std::env::set_var("ZAI_API_KEY", "operator-key");
        }
        assert!(
            !workflow_runtime_credentials_available(
                r#"{"OPENCODE_MODEL_PROVIDER":"zai-coding-plan"}"#
            )
            .unwrap()
        );

        // SAFETY: restore previous process environment for the next test.
        unsafe {
            match old {
                Some(value) => std::env::set_var("ZAI_API_KEY", value),
                None => std::env::remove_var("ZAI_API_KEY"),
            }
        }
    }

    // ── extract_host_port ───────────────────────────────────────────────

    fn make_port_map(port: u16, host_port: &str) -> HashMap<String, Option<Vec<PortBinding>>> {
        let mut map = HashMap::new();
        map.insert(
            format!("{port}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some(host_port.to_string()),
            }]),
        );
        map
    }

    #[test]
    fn extract_host_port_valid() {
        let ports = make_port_map(3000, "32768");
        let result = extract_host_port(&ports, 3000).unwrap();
        assert_eq!(result, 32768);
    }

    #[test]
    fn extract_host_port_missing_port() {
        let ports = make_port_map(3000, "32768");
        let result = extract_host_port(&ports, 8080);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_invalid_number() {
        let ports = make_port_map(3000, "not-a-number");
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_zero_is_not_ready() {
        let ports = make_port_map(3000, "0");
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_empty_bindings() {
        let mut ports: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        ports.insert("3000/tcp".to_string(), Some(vec![]));
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    // ── extract_ports (full) ────────────────────────────────────────────

    fn make_inspect(
        port_map: HashMap<String, Option<Vec<PortBinding>>>,
    ) -> ContainerInspectResponse {
        ContainerInspectResponse {
            network_settings: Some(NetworkSettings {
                ports: Some(port_map),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extract_ports_no_ssh() {
        let ports = make_port_map(3000, "49000");
        let inspect = make_inspect(ports);
        let (sidecar, ssh) = extract_ports(&inspect, 3000, false).unwrap();
        assert_eq!(sidecar, 49000);
        assert!(ssh.is_none());
    }

    #[test]
    fn extract_ports_with_ssh() {
        let mut ports = make_port_map(3000, "49000");
        ports.insert(
            format!("{DEFAULT_SIDECAR_SSH_PORT}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some("49001".to_string()),
            }]),
        );
        let inspect = make_inspect(ports);
        let (sidecar, ssh) = extract_ports(&inspect, 3000, true).unwrap();
        assert_eq!(sidecar, 49000);
        assert_eq!(ssh, Some(49001));
    }

    #[test]
    fn extract_ports_missing_network() {
        let inspect = ContainerInspectResponse {
            network_settings: None,
            ..Default::default()
        };
        let result = extract_ports(&inspect, 3000, false);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_retries_until_success() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = retry_port_mapping_lookup_inner("test resolution", "ctr-1", 3, 0, {
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();
                async move {
                    let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if attempt < 2 {
                        Err(SandboxError::Docker(
                            "Host port for 3000/tcp is not assigned yet".into(),
                        ))
                    } else {
                        Ok(49000u16)
                    }
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(result, 49000);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_stops_on_non_retryable_error() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result =
            retry_port_mapping_lookup_inner::<u16, _, _>("test resolution", "ctr-2", 3, 0, {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(SandboxError::Docker(
                            "Failed to connect to Docker: daemon unavailable".into(),
                        ))
                    }
                }
            })
            .await;

        let err = result.expect_err("expected non-retryable error to bubble up");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            err.to_string().contains("daemon unavailable"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_wraps_exhausted_transient_error() {
        let result = retry_port_mapping_lookup_inner::<u16, _, _>(
            "test resolution",
            "ctr-3",
            2,
            0,
            || async {
                Err(SandboxError::Docker(
                    "Missing host port for 3000/tcp".into(),
                ))
            },
        )
        .await;

        let err = result.expect_err("expected retries to exhaust");
        assert!(
            err.to_string().contains(
                "test resolution failed: Docker did not publish sidecar port for container ctr-3 after 2 attempts"
            ),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("Missing host port for 3000/tcp"),
            "unexpected error: {err}"
        );
    }

    // ── SandboxState ────────────────────────────────────────────────────

    #[test]
    fn sandbox_state_default_is_running() {
        assert_eq!(SandboxState::default(), SandboxState::Running);
    }

    #[test]
    fn sandbox_state_serialization_roundtrip() {
        let running = serde_json::to_string(&SandboxState::Running).unwrap();
        let stopped = serde_json::to_string(&SandboxState::Stopped).unwrap();
        assert_eq!(
            serde_json::from_str::<SandboxState>(&running).unwrap(),
            SandboxState::Running
        );
        assert_eq!(
            serde_json::from_str::<SandboxState>(&stopped).unwrap(),
            SandboxState::Stopped
        );
    }
}

/// Single-pass store admission scan (admission.rs): one `values()` walk must
/// reproduce exactly what the former two dedicated scans computed — the
/// count-check inputs and the memory-budget inputs.
#[cfg(test)]
mod admission_scan_tests {
    use super::*;

    fn record(id: &str, state: SandboxState, memory_mb: u64) -> SandboxRecord {
        SandboxRecord {
            id: id.into(),
            container_id: format!("ctr-{id}"),
            sidecar_url: "http://127.0.0.1:0".into(),
            sidecar_port: 0,
            ssh_port: None,
            token: "t".into(),
            created_at: 0,
            cpu_cores: 1,
            memory_mb,
            state,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 0,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: String::new(),
            base_env_json: String::new(),
            user_env_json: String::new(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
            service_id: None,
            tee_config: None,
            extra_ports: HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
            capabilities_json: String::new(),
        }
    }

    #[test]
    fn scan_empty_store() {
        let scan = scan_records_for_admission(&[], None);
        assert_eq!(scan.total_count, 0);
        assert!(!scan.reusing_existing_slot);
        assert!(scan.running_memory_mb.is_empty());
    }

    #[test]
    fn scan_counts_all_rows_but_sums_only_running_memory() {
        let records = vec![
            record("a", SandboxState::Running, 1024),
            record("b", SandboxState::Stopped, 2048),
            record("c", SandboxState::Running, 512),
        ];
        let scan = scan_records_for_admission(&records, None);
        // Count cap sees every row (stopped sandboxes hold store slots)…
        assert_eq!(scan.total_count, 3);
        // …the budget sees only running footprints.
        assert_eq!(scan.running_memory_mb, vec![1024, 512]);
        assert!(!scan.reusing_existing_slot);
    }

    #[test]
    fn scan_running_unlimited_rows_stay_visible() {
        // memory_mb=0 rows must remain in the vec so check_host_memory_budget
        // keeps its accounted-at-max / unaccounted-warning semantics.
        let records = vec![record("a", SandboxState::Running, 0)];
        let scan = scan_records_for_admission(&records, None);
        assert_eq!(scan.running_memory_mb, vec![0]);
    }

    #[test]
    fn scan_reused_id_counts_slot_but_frees_memory() {
        let records = vec![
            record("a", SandboxState::Running, 1024),
            record("b", SandboxState::Running, 2048),
        ];
        let scan = scan_records_for_admission(&records, Some("b"));
        // The replaced record still occupies a store slot (the count check
        // then subtracts it via reusing_existing_slot)…
        assert_eq!(scan.total_count, 2);
        assert!(scan.reusing_existing_slot);
        // …but its container's memory is freed by the recreate.
        assert_eq!(scan.running_memory_mb, vec![1024]);
    }

    #[test]
    fn scan_reused_id_flagged_even_when_stopped() {
        // Recreating a STOPPED sandbox also reuses its slot: the former
        // count check keyed off store presence (`get(id).is_some()`), not
        // state, and the stopped row never contributed running memory.
        let records = vec![record("a", SandboxState::Stopped, 1024)];
        let scan = scan_records_for_admission(&records, Some("a"));
        assert_eq!(scan.total_count, 1);
        assert!(scan.reusing_existing_slot);
        assert!(scan.running_memory_mb.is_empty());
    }

    #[test]
    fn scan_absent_reused_id_sets_no_flag() {
        // A fresh sandbox id (no override, or an override that was never
        // stored) must not claim slot reuse — matches `get(id) == None`.
        let records = vec![record("a", SandboxState::Running, 1024)];
        let scan = scan_records_for_admission(&records, Some("zz"));
        assert!(!scan.reusing_existing_slot);
        assert_eq!(scan.running_memory_mb, vec![1024]);
    }

    /// Differential check: the single pass must equal the legacy two-pass
    /// computation (count scan + filtered memory scan) row-for-row across a
    /// matrix of store shapes and reuse targets.
    #[test]
    fn scan_matches_legacy_two_pass_semantics() {
        let stores: Vec<Vec<SandboxRecord>> = vec![
            vec![],
            vec![record("a", SandboxState::Running, 0)],
            vec![
                record("a", SandboxState::Running, 1024),
                record("b", SandboxState::Stopped, 2048),
                record("c", SandboxState::Running, 0),
                record("d", SandboxState::Running, 4096),
            ],
        ];
        for records in &stores {
            for reused in [None, Some("a"), Some("b"), Some("missing")] {
                // Legacy pass 1: count = all rows; reuse = presence by id.
                let legacy_count = records.len();
                let legacy_reusing = records.iter().any(|r| reused == Some(r.id.as_str()));
                // Legacy pass 2: running memory, reused id excluded.
                let legacy_memory: Vec<u64> = records
                    .iter()
                    .filter(|r| r.state == SandboxState::Running)
                    .filter(|r| reused != Some(r.id.as_str()))
                    .map(|r| r.memory_mb)
                    .collect();

                let scan = scan_records_for_admission(records, reused);
                assert_eq!(scan.total_count, legacy_count, "reused={reused:?}");
                assert_eq!(
                    scan.reusing_existing_slot, legacy_reusing,
                    "reused={reused:?}"
                );
                assert_eq!(scan.running_memory_mb, legacy_memory, "reused={reused:?}");
            }
        }
    }
}

/// Invariants of the merged workspace-bootstrap exec (docker_create.rs).
/// The merge collapses two post-start exec round-trips into one; these pin
/// the properties that make it semantically equal to the pre-merge pair.
#[cfg(test)]
mod workspace_bootstrap_tests {
    use super::*;

    const CONFIG_DIR: &str = "/home/agent/.opencode-home/.config";

    #[test]
    fn merged_command_mkdirs_before_chown() {
        // Repair-path precondition: /home/agent is root-owned, so root's
        // mkdir must run BEFORE the chown hands the tree to agent (after
        // which cap_drop=ALL root has no DAC_OVERRIDE to write into it).
        let mkdir_at = WORKSPACE_BOOTSTRAP_ROOT_CMD
            .find("mkdir -p")
            .expect("merged command must create the opencode dirs");
        let chown_at = WORKSPACE_BOOTSTRAP_ROOT_CMD
            .find("chown -R agent:agent /home/agent")
            .expect("merged command must repair workspace ownership");
        assert!(
            mkdir_at < chown_at,
            "mkdir must precede chown: {WORKSPACE_BOOTSTRAP_ROOT_CMD}"
        );
    }

    #[test]
    fn merged_command_drops_to_agent_when_root_mkdir_denied() {
        // Canonical-image case (agent-owned /home/agent, dirs absent): root's
        // mkdir is denied under cap_drop=ALL, so the merged exec must retry
        // as the agent user via su, targeting the same directory, BEFORE the
        // chown (the decision keys off the tree's CURRENT owner).
        let su_at = WORKSPACE_BOOTSTRAP_ROOT_CMD
            .find(&format!("su agent -s /bin/sh -c 'mkdir -p {CONFIG_DIR}'"))
            .expect("merged command must retry the mkdir as the agent user");
        let chown_at = WORKSPACE_BOOTSTRAP_ROOT_CMD
            .find("chown -R agent:agent /home/agent")
            .expect("merged command must repair workspace ownership");
        assert!(
            su_at < chown_at,
            "su fallback must precede chown: {WORKSPACE_BOOTSTRAP_ROOT_CMD}"
        );
    }

    #[test]
    fn merged_command_chown_is_unconditional_and_tolerant() {
        // The pre-merge chown exec ran regardless of any mkdir outcome and
        // tolerated failure (`|| true`). The merged form must keep both:
        // `;` separators (not `&&`) so chown runs even when mkdir fails,
        // and `|| true` so a chown failure doesn't change the exit path.
        assert!(
            WORKSPACE_BOOTSTRAP_ROOT_CMD
                .contains("chown -R agent:agent /home/agent 2>/dev/null || true"),
            "chown must stay best-effort: {WORKSPACE_BOOTSTRAP_ROOT_CMD}"
        );
        assert!(
            !WORKSPACE_BOOTSTRAP_ROOT_CMD.contains("&&"),
            "stages must be `;`-separated so each runs unconditionally: {WORKSPACE_BOOTSTRAP_ROOT_CMD}"
        );
    }

    #[test]
    fn merged_command_exit_code_reports_dir_existence() {
        // The caller's fallback decision keys off the exit code, so the
        // command must END with the `test -d` verification of the exact
        // directory the fallback would create.
        assert!(
            WORKSPACE_BOOTSTRAP_ROOT_CMD
                .trim_end()
                .ends_with(&format!("test -d {CONFIG_DIR}")),
            "merged command must end with the dir verification: {WORKSPACE_BOOTSTRAP_ROOT_CMD}"
        );
    }

    #[test]
    fn fallback_matches_pre_merge_agent_exec() {
        // The fallback IS the pre-merge second exec: same mkdir, same target,
        // run as the agent user (asserted at the call site), and no chown —
        // the agent user cannot chown and never needed to.
        assert_eq!(
            WORKSPACE_BOOTSTRAP_AGENT_FALLBACK_CMD,
            format!("mkdir -p {CONFIG_DIR}")
        );
        assert!(!WORKSPACE_BOOTSTRAP_AGENT_FALLBACK_CMD.contains("chown"));
    }

    #[test]
    fn merged_and_fallback_target_the_same_directory() {
        assert!(WORKSPACE_BOOTSTRAP_ROOT_CMD.contains(CONFIG_DIR));
        assert!(WORKSPACE_BOOTSTRAP_AGENT_FALLBACK_CMD.contains(CONFIG_DIR));
    }
}
