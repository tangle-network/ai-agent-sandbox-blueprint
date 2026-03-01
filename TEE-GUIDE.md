# TEE Operator & Developer Guide

This guide covers deploying, configuring, and verifying TEE (Trusted Execution Environment) sandboxes with the AI Agent Sandbox Blueprint.

## Overview

TEE mode provides hardware-enforced isolation for sandbox workloads:

- **Attestation** — Cryptographic proof that the sandbox runs inside a genuine enclave with the expected code measurement
- **Sealed secrets** — Secrets encrypted to the enclave's public key, decryptable only inside the TEE
- **Measurement verification** — Clients can verify the sidecar image hash against the on-chain attestation before trusting the sandbox

TEE is enforced at the contract level: when `teeRequired=true`, the Solidity `_handleProvisionResult` reverts with `MissingTeeAttestation` if the operator's provision result contains an empty attestation.

## Prerequisites

Choose a TEE backend based on your infrastructure:

| Backend | TEE Type | Hardware | Managed? | Feature Flag |
|---------|----------|----------|----------|--------------|
| **Phala** | TDX | Phala Cloud CVMs | Yes | `tee-phala` |
| **AWS Nitro** | Nitro Enclaves | EC2 (c5/c6i/m5) | Semi | `tee-aws-nitro` |
| **GCP** | TDX or SEV-SNP | Confidential Space | Semi | `tee-gcp` |
| **Azure** | SEV-SNP | DCasv5/ECasv5 VMs | Semi | `tee-azure` |
| **Direct** | TDX, SEV-SNP, or Nitro | Your hardware | No | `tee-direct` |

**Managed** = the backend provisions cloud VMs/containers on your behalf.
**Direct** = you run the Docker daemon on TEE-capable hardware with device passthrough.

## Building

```bash
# All backends
cargo build -p ai-agent-tee-instance-blueprint-bin --features tee-all

# Specific backend
cargo build -p ai-agent-tee-instance-blueprint-bin --features tee-phala
cargo build -p ai-agent-tee-instance-blueprint-bin --features tee-direct
```

## Configuration

### Common

| Env Var | Description | Required |
|---------|-------------|----------|
| `TEE_BACKEND` | Backend selection: `phala`, `nitro`, `aws`, `gcp`, `azure`, `direct` | Yes |

### Phala dstack

| Env Var | Description | Default |
|---------|-------------|---------|
| `PHALA_API_KEY` | Phala Cloud API key | Required |
| `PHALA_API_ENDPOINT` | Phala Cloud API URL | `https://cloud.phala.network` |

### AWS Nitro Enclaves

| Env Var | Description | Default |
|---------|-------------|---------|
| `AWS_REGION` | AWS region | `us-east-1` |
| `AWS_NITRO_AMI_ID` | AMI with Nitro Enclaves support + EIF | Required |
| `AWS_NITRO_SUBNET_ID` | VPC subnet for the instance | Required |
| `AWS_NITRO_SECURITY_GROUP_ID` | Security group allowing sidecar port | Required |
| `AWS_NITRO_INSTANCE_TYPE` | EC2 instance type | `c5.xlarge` |
| `AWS_NITRO_KMS_KEY_ID` | KMS key for sealed secrets | Optional |
| `AWS_NITRO_IAM_INSTANCE_PROFILE` | IAM instance profile ARN | Optional |

### GCP Confidential Space

| Env Var | Description | Default |
|---------|-------------|---------|
| `GCP_PROJECT_ID` | GCP project ID | Required |
| `GCP_ZONE` | Compute Engine zone | `us-central1-a` |
| `GCP_CONFIDENTIAL_SPACE_IMAGE` | Confidential Space VM image | Required |
| `GCP_MACHINE_TYPE` | Machine type (`n2d-*` = SEV-SNP, `c3-*` = TDX) | `n2d-standard-4` |
| `GCP_SERVICE_ACCOUNT_EMAIL` | Service account for the VM | Optional |
| `GCP_NETWORK` | VPC network | `default` |
| `GCP_SUBNET` | VPC subnet | `default` |
| `GCP_KMS_KEY_RESOURCE` | Cloud KMS key for sealed secrets | Optional |

### Azure Confidential VMs

| Env Var | Description | Default |
|---------|-------------|---------|
| `AZURE_SUBSCRIPTION_ID` | Azure subscription | Required |
| `AZURE_RESOURCE_GROUP` | Resource group | Required |
| `AZURE_LOCATION` | Azure region | `eastus` |
| `AZURE_VM_IMAGE` | Confidential VM image URN | Required |
| `AZURE_VM_SIZE` | VM size (`DCasv5` = SEV-SNP, `DCesv5` = TDX) | `Standard_DC4as_v5` |
| `AZURE_SUBNET_ID` | VNet subnet resource ID | Required |
| `AZURE_TENANT_ID` | Azure AD tenant for OAuth2 | Required |
| `AZURE_CLIENT_ID` | Service principal client ID | Required |
| `AZURE_CLIENT_SECRET` | Service principal secret | Required |
| `AZURE_KEY_VAULT_URL` | Key Vault URL for sealed secrets (SKR) | Optional |
| `AZURE_MAA_ENDPOINT` | Microsoft Azure Attestation endpoint | Optional |

### Direct (Operator Hardware)

| Env Var | Description | Default |
|---------|-------------|---------|
| `TEE_DIRECT_TYPE` | TEE type: `tdx`, `sev`, or `nitro` | Required |

The host must have the corresponding device node available:

| TEE Type | Device Node |
|----------|-------------|
| TDX | `/dev/tdx_guest` |
| SEV-SNP | `/dev/sev-guest` |
| Nitro | `/dev/nsm` |

## Contract Deployment

Deploy with `teeRequired=true`:

```bash
forge script script/DeployTeeInstance.s.sol --broadcast --rpc-url $RPC_URL
```

The contract enforces:
- `_handleProvisionResult` reverts with `MissingTeeAttestation` if `teeRequired && attestationJson.length == 0`
- Attestation hash stored on-chain: `keccak256(attestationJsonBytes)` in `getAttestationHash(serviceId, operator)`

## Provisioning Flow

```
User                     Contract                  Operator
  |                         |                         |
  |-- requestService() ---->|                         |
  |                         |-- onServiceInitialized->|
  |                         |                         |
  |                         |          auto-provision starts:
  |                         |          1. Read serviceConfig from BSM
  |                         |          2. Deploy sidecar via TEE backend
  |                         |          3. Wait for health + attestation
  |                         |          4. Submit JOB_PROVISION on-chain
  |                         |                         |
  |                         |<-- provisionResult -----|
  |                         |    (sandbox_id,         |
  |                         |     sidecar_url,        |
  |                         |     tee_attestation_json,
  |                         |     tee_public_key_json) |
  |                         |                         |
  |                         | _handleProvisionResult: |
  |                         |   verify tee_attestation |
  |                         |   store attestation hash |
  |                         |                         |
  |<-- service ready -------|                         |
```

## Operator API Endpoints

All endpoints require PASETO session auth (`Authorization: Bearer <token>`).

### `GET /api/sandboxes/{id}/tee/attestation`

Fetch fresh attestation from a running TEE sandbox.

**Response (200):**
```json
{
  "tee_type": "Tdx",
  "evidence": [/* raw bytes */],
  "measurement": [/* raw bytes */],
  "timestamp": 1700000000
}
```

### `GET /api/sandboxes/{id}/tee/public-key`

Derive a TEE-bound public key for sealed secret encryption.

**Response (200):**
```json
{
  "algorithm": "x25519-hkdf-sha256",
  "public_key_bytes": [/* raw bytes */],
  "attestation": { /* AttestationReport */ }
}
```

### `POST /api/sandboxes/{id}/tee/sealed-secrets`

Inject encrypted secrets into a TEE sandbox.

**Request:**
```json
{
  "algorithm": "x25519-xsalsa20-poly1305",
  "ciphertext": [/* encrypted bytes */],
  "nonce": [/* nonce bytes */]
}
```

**Response (200):**
```json
{
  "success": true,
  "secrets_count": 3,
  "error": null
}
```

## Sidecar TEE API Contract

The sidecar image must implement these endpoints for TEE functionality:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/tee/attestation` | GET | Return hardware attestation report |
| `/tee/public-key` | GET | Derive TEE-bound public key |
| `/tee/sealed-secrets` | POST | Decrypt and inject sealed secrets |

### `GET /tee/attestation`

Returns an `AttestationReport`:
```json
{
  "tee_type": "Tdx",
  "evidence": [/* TDX TDREPORT / Nitro NSM doc / SEV-SNP report */],
  "measurement": [/* MRTD / PCR values / LAUNCH_DIGEST */],
  "timestamp": 1700000000
}
```

### `GET /tee/public-key`

Returns a `TeePublicKey` bound to the enclave measurement:
```json
{
  "algorithm": "x25519-hkdf-sha256",
  "public_key_bytes": [1, 2, 3, ...],
  "attestation": { /* fresh AttestationReport */ }
}
```

### `POST /tee/sealed-secrets`

Accepts a `SealedSecret`, decrypts inside the enclave, and injects into the environment:
```json
{
  "success": true,
  "secrets_count": 3,
  "error": null
}
```

## Client Attestation Verification

### 1. Read the attestation

From on-chain `ProvisionOutput.tee_attestation_json`, or from the operator API:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  https://operator/api/sandboxes/$ID/tee/attestation
```

### 2. Verify the on-chain hash

```solidity
bytes32 expected = keccak256(bytes(attestationJson));
bytes32 stored = blueprint.getAttestationHash(serviceId, operator);
require(expected == stored, "Attestation mismatch");
```

### 3. Verify the evidence

The evidence format depends on the TEE type:

**Intel TDX:**
- Parse the TDREPORT (1024 bytes)
- Extract MRTD at offset 512 (48 bytes, SHA-384)
- Compare MRTD against the expected sidecar image measurement

**AWS Nitro:**
- Parse the NSM attestation document (CBOR-encoded)
- Verify PCR values against expected enclave image hash

**AMD SEV-SNP:**
- Parse the ATTESTATION_REPORT
- Extract LAUNCH_DIGEST at offset 0x90 (48 bytes, SHA-384)
- Compare against expected VM image measurement

**Phala:**
- `tcb_info` and `app_certificates` are Phala-specific
- Verify against Phala's attestation service

> **Note:** No client-side verification library is included in this repo. The above is a reference for building one.

## Troubleshooting

### `MissingTeeAttestation` revert

The on-chain contract requires `tee_attestation_json` to be non-empty when `teeRequired=true`. Causes:
- Operator's TEE backend failed to generate attestation
- The idempotent provision path returned empty attestation (fixed in this version)

Check operator logs for `TEE public key derivation failed` warnings.

### `TEE backend not initialized`

The operator binary hasn't called `init_tee_backend()` at startup. Verify:
- `TEE_BACKEND` env var is set
- The correct feature flag is enabled in the build
- Backend-specific env vars are configured

### Device not found (`/dev/tdx_guest`, `/dev/sev-guest`)

Direct backend only. The host doesn't have TEE hardware or the device node isn't accessible:
- Verify you're on TEE-capable hardware
- Check device permissions: `ls -la /dev/tdx_guest`
- Docker needs `--device /dev/tdx_guest:/dev/tdx_guest` — the Direct backend handles this automatically

### Container fails to start with device mapping

Docker refuses to create a container with a device that doesn't exist on the host. For testing without TEE hardware, use the `new_without_device` constructor (see Testing section below).

## Local Development & Testing

### Unit tests (MockTeeBackend, no setup needed)

```bash
cargo test -p sandbox-runtime --features tee-all
cargo test -p ai-agent-tee-instance-blueprint-lib
```

These use `MockTeeBackend` — no Docker, no hardware, no network. Good for logic testing.

### Integration tests (Direct backend without TEE hardware)

```bash
TEE_INTEGRATION=1 cargo test -p sandbox-runtime --features tee-all,test-utils -- tee_integration
TEE_INTEGRATION=1 cargo test -p ai-agent-tee-instance-blueprint-lib -- tee_integration
```

Uses `DirectTeeBackend::new_without_device()` which skips the TEE device passthrough in `build_config()`. Exercises real Docker orchestration, port extraction, health checks — everything except the native ioctl.

**Requirements:** Docker daemon running, sidecar image available.

### Integration tests with TEE hardware

On a TEE-capable host with `/dev/tdx_guest` or `/dev/sev-guest`:

```bash
TEE_DIRECT_TYPE=tdx TEE_INTEGRATION=1 cargo test -p sandbox-runtime --features tee-all,test-utils -- tee_integration
```

### Cloud backend tests

Set the backend-specific credentials and:

```bash
TEE_BACKEND=phala TEE_INTEGRATION=1 cargo test ...
```
