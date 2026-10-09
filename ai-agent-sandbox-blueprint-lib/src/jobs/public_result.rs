//! Explicit allowlist for public, permanently recorded create results.

use serde_json::{Value, json};

/// Shared by single and batch create. Only public fields can enter this
/// projection; never serialize a SandboxRecord (which contains bearer secrets).
pub(super) fn sandbox_created(id: &str, sidecar_url: &str, ssh_port: Option<u16>) -> Value {
    json!({
        "sandboxId": id,
        "sidecarUrl": sidecar_url,
        "sshPort": ssh_port,
        "credentialsDelivery": "operator-api-v1",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SandboxCreateOutput;
    use blueprint_sdk::alloy::sol_types::SolValue;

    #[test]
    fn public_create_contract_matches_shared_fixture() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/create-result-operator-api-v1.json"
        ))
        .unwrap();
        let mut result = sandbox_created("sandbox-fixture", "https://sidecar.example", Some(2222));
        result["teeAttestationJson"] = json!("");
        result["teePublicKeyJson"] = json!("");
        let expected: Value = serde_json::from_str(fixture["json"].as_str().unwrap()).unwrap();
        assert_eq!(result, expected);
        assert!(result.get("token").is_none());

        // Pin the existing tuple-wrapped ABI, usable unchanged by TS/Python.
        // JSON key ordering is not part of the contract.
        let encoded = SandboxCreateOutput {
            sandboxId: fixture["sandboxId"].as_str().unwrap().into(),
            json: fixture["json"].as_str().unwrap().into(),
        }
        .abi_encode();
        let expected_hex = fixture["abiEncoded"].as_str().unwrap();
        let actual_hex: String = encoded.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(format!("0x{actual_hex}"), expected_hex);
    }
}
