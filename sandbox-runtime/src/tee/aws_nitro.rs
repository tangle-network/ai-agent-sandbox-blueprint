//! AWS Nitro Enclaves TEE backend.
//!
//! Deploys sidecar containers inside Nitro Enclaves on EC2 instances.
//! The enclave runs in complete isolation — no networking, no persistent
//! storage, no interactive access. Communication is via vsock only.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │  Parent EC2 Instance (EnclaveOptions=true)  │
//! │  ┌─────────────────┐  ┌──────────────────┐ │
//! │  │  vsock-proxy    │  │  TCP listener    │ │
//! │  │  KMS via vsock  │  │  forwards to     │ │
//! │  │                 │  │  enclave vsock   │ │
//! │  └────────┬────────┘  └──────┬───────────┘ │
//! │           │ vsock             │ vsock       │
//! │  ┌────────┴───────────────────┴──────────┐ │
//! │  │         Nitro Enclave (CID 16)        │ │
//! │  │  ┌──────────────────────────────────┐ │ │
//! │  │  │  Sidecar (HTTP on vsock:port)    │ │ │
//! │  │  │  - NSM attestation via /dev/nsm  │ │ │
//! │  │  │  - KMS decrypt via vsock proxy   │ │ │
//! │  │  └──────────────────────────────────┘ │ │
//! │  │  No network, no disk, no shell        │ │
//! │  └───────────────────────────────────────┘ │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Sealed secrets
//!
//! The enclave generates an ephemeral RSA-2048 key pair and embeds the public
//! key in the NSM attestation document. KMS condition keys
//! (`kms:RecipientAttestation:PCR0`) ensure only attested enclaves can decrypt.
//! The `RecipientInfo` on KMS Decrypt re-encrypts the plaintext to the
//! enclave's key — the operator never sees secrets.

use std::time::Duration;

use aws_sdk_ec2::Client as Ec2Client;
use aws_sdk_ec2::types::{
    EnclaveOptionsRequest, IamInstanceProfileSpecification, InstanceStateName,
};
use base64::Engine;
use tokio::sync::OnceCell;

use super::sealed_secrets::{SealedSecret, SealedSecretResult, TeePublicKey};
use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};

/// Configuration for the AWS Nitro backend, read from environment variables.
#[derive(Clone, Debug)]
pub struct NitroConfig {
    pub region: String,
    pub subnet_id: String,
    pub security_group_id: String,
    pub ami_id: String,
    pub instance_type: String,
    pub kms_key_id: Option<String>,
    pub iam_instance_profile: Option<String>,
}

impl NitroConfig {
    /// Load configuration from environment variables.
    ///
    /// Required: `AWS_REGION`, `AWS_NITRO_SUBNET_ID`, `AWS_NITRO_SECURITY_GROUP_ID`,
    /// `AWS_NITRO_AMI_ID`.
    /// Optional: `AWS_NITRO_INSTANCE_TYPE` (default: c5.xlarge),
    /// `AWS_NITRO_KMS_KEY_ID`, `AWS_NITRO_IAM_INSTANCE_PROFILE`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            region: require_env("AWS_REGION")?,
            subnet_id: require_env("AWS_NITRO_SUBNET_ID")?,
            security_group_id: require_env("AWS_NITRO_SECURITY_GROUP_ID")?,
            ami_id: require_env("AWS_NITRO_AMI_ID")?,
            instance_type: std::env::var("AWS_NITRO_INSTANCE_TYPE")
                .unwrap_or_else(|_| "c5.xlarge".to_string()),
            kms_key_id: std::env::var("AWS_NITRO_KMS_KEY_ID").ok(),
            iam_instance_profile: std::env::var("AWS_NITRO_IAM_INSTANCE_PROFILE").ok(),
        })
    }
}

/// TEE backend that deploys containers inside AWS Nitro Enclaves.
pub struct NitroBackend {
    pub config: NitroConfig,
    ec2: OnceCell<Ec2Client>,
}

impl NitroBackend {
    pub fn new(config: NitroConfig) -> Self {
        Self {
            config,
            ec2: OnceCell::new(),
        }
    }

    /// Lazily initialize the EC2 client (loads AWS config from environment).
    async fn ec2(&self) -> &Ec2Client {
        self.ec2
            .get_or_init(|| async {
                let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_sdk_ec2::config::Region::new(self.config.region.clone()))
                    .load()
                    .await;
                Ec2Client::new(&aws_config)
            })
            .await
    }

    /// Build the user-data script that configures the enclave on the parent EC2 instance.
    ///
    /// The AMI must have `aws-nitro-enclaves-cli` pre-installed and the sidecar
    /// EIF at `/opt/enclave/sidecar.eif`.
    fn build_user_data(&self, params: &TeeDeployParams) -> String {
        let mut script = String::from("#!/bin/bash\nset -ex\n\n");

        // Configure the Nitro Enclaves allocator with requested resources.
        script.push_str(&format!(
            "cat > /etc/nitro_enclaves/allocator.yaml << 'EOF'\n\
             ---\n\
             memory_mib: {}\n\
             cpu_count: {}\n\
             EOF\n\
             systemctl restart nitro-enclaves-allocator.service\n\n",
            params.memory_mb.max(512),
            params.cpu_cores.max(2),
        ));

        // Start a TCP-to-vsock forwarder so the operator can reach the sidecar
        // HTTP API on the parent's public IP. socat bridges TCP on the parent
        // to vsock on the enclave (CID 16).
        script.push_str(&format!(
            "nohup socat TCP4-LISTEN:{port},fork,reuseaddr VSOCK-CONNECT:16:{port} &\n\n",
            port = params.http_port,
        ));

        if let Some(ssh_port) = params.ssh_port {
            script.push_str(&format!(
                "nohup socat TCP4-LISTEN:{ssh_port},fork,reuseaddr VSOCK-CONNECT:16:22 &\n\n",
            ));
        }

        // Write env vars as JSON for the enclave to read via vsock.
        let env_map: serde_json::Map<String, serde_json::Value> = params
            .env_vars
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        script.push_str("mkdir -p /opt/enclave\n");
        script.push_str("cat > /opt/enclave/env.json << 'ENVEOF'\n");
        script.push_str(&serde_json::to_string_pretty(&env_map).unwrap_or_default());
        script.push_str("\nENVEOF\n\n");

        // Launch the enclave from the pre-baked EIF.
        script.push_str(&format!(
            "nitro-cli run-enclave \\\n\
             \t--cpu-count {} \\\n\
             \t--memory {} \\\n\
             \t--eif-path /opt/enclave/sidecar.eif \\\n\
             \t--enclave-cid 16\n",
            params.cpu_cores.max(2),
            params.memory_mb.max(512),
        ));

        script
    }

    /// Poll EC2 DescribeInstances until the instance reaches `running` state,
    /// then return its public IP address.
    async fn wait_for_running(&self, instance_id: &str) -> Result<String> {
        let ec2 = self.ec2().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(SandboxError::CloudProvider(format!(
                    "EC2 instance {instance_id} did not reach running state within timeout"
                )));
            }

            let desc = ec2
                .describe_instances()
                .instance_ids(instance_id)
                .send()
                .await
                .map_err(|e| SandboxError::CloudProvider(format!("DescribeInstances: {e}")))?;

            if let Some(instance) = desc
                .reservations()
                .first()
                .and_then(|r| r.instances().first())
            {
                if let Some(state) = instance.state() {
                    if state.name() == Some(&InstanceStateName::Running) {
                        return instance
                            .public_ip_address()
                            .map(|ip| ip.to_string())
                            .ok_or_else(|| {
                                SandboxError::CloudProvider(
                                    "No public IP assigned to instance".into(),
                                )
                            });
                    }
                    if matches!(
                        state.name(),
                        Some(&InstanceStateName::Terminated | &InstanceStateName::ShuttingDown)
                    ) {
                        return Err(SandboxError::CloudProvider(format!(
                            "EC2 instance {instance_id} entered terminal state"
                        )));
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

#[async_trait::async_trait]
impl TeeBackend for NitroBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let ec2 = self.ec2().await;

        // Base64-encode user data (EC2 requirement).
        let user_data = self.build_user_data(params);
        let user_data_b64 =
            base64::engine::general_purpose::STANDARD.encode(user_data.as_bytes());

        // Launch EC2 instance with enclave support.
        let mut run_req = ec2
            .run_instances()
            .image_id(&self.config.ami_id)
            .instance_type(aws_sdk_ec2::types::InstanceType::from(
                self.config.instance_type.as_str(),
            ))
            .min_count(1)
            .max_count(1)
            .security_group_ids(&self.config.security_group_id)
            .subnet_id(&self.config.subnet_id)
            .user_data(&user_data_b64)
            .enclave_options(EnclaveOptionsRequest::builder().enabled(true).build());

        if let Some(ref profile_arn) = self.config.iam_instance_profile {
            run_req = run_req.iam_instance_profile(
                IamInstanceProfileSpecification::builder()
                    .arn(profile_arn)
                    .build(),
            );
        }

        let run_resp = run_req
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("EC2 RunInstances: {e}")))?;

        let instance_id = run_resp
            .instances()
            .first()
            .and_then(|i| i.instance_id())
            .ok_or_else(|| SandboxError::CloudProvider("No instance ID returned".into()))?
            .to_string();

        // Wait for EC2 instance to be running and get its public IP.
        let public_ip = self.wait_for_running(&instance_id).await?;
        let sidecar_url = format!("http://{}:{}", public_ip, params.http_port);

        // Wait for sidecar to be healthy inside the enclave.
        super::wait_for_sidecar_health(
            &sidecar_url,
            &params.sidecar_token,
            Duration::from_secs(300),
        )
        .await?;

        // Fetch attestation from the sidecar.
        let attestation =
            super::fetch_sidecar_attestation(&sidecar_url, &params.sidecar_token).await?;

        let metadata = serde_json::json!({
            "ec2_instance_id": instance_id,
            "public_ip": public_ip,
            "region": self.config.region,
            "instance_type": self.config.instance_type,
        });

        Ok(TeeDeployment {
            deployment_id: instance_id,
            sidecar_url,
            ssh_port: params.ssh_port,
            attestation,
            metadata_json: metadata.to_string(),
        })
    }

    async fn attestation(&self, deployment_id: &str) -> Result<AttestationReport> {
        let (sidecar_url, token) = super::sidecar_info_for_deployment(deployment_id)?;
        super::fetch_sidecar_attestation(&sidecar_url, &token).await
    }

    async fn stop(&self, deployment_id: &str) -> Result<()> {
        self.ec2()
            .await
            .stop_instances()
            .instance_ids(deployment_id)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("EC2 StopInstances: {e}")))?;
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        self.ec2()
            .await
            .terminate_instances()
            .instance_ids(deployment_id)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("EC2 TerminateInstances: {e}")))?;
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        TeeType::Nitro
    }

    async fn derive_public_key(&self, deployment_id: &str) -> Result<TeePublicKey> {
        super::sidecar_derive_public_key(deployment_id).await
    }

    async fn inject_sealed_secrets(
        &self,
        deployment_id: &str,
        sealed: &SealedSecret,
    ) -> Result<SealedSecretResult> {
        super::sidecar_inject_sealed_secrets(deployment_id, sealed).await
    }
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| {
        SandboxError::Validation(format!(
            "AWS Nitro backend requires {name} environment variable"
        ))
    })
}
